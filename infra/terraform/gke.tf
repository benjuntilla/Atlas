# GKE cluster.
#
# Standard rather than Autopilot, for two specific reasons:
#
#   1. Dataplane V2 is set explicitly here. The NetworkPolicies in
#      infra/k8s are load-bearing — they are what makes "only the gateway
#      can reach geo-engine" true rather than aspirational — and a cluster
#      without an enforcing CNI accepts those objects and silently ignores
#      them. Making the datapath an explicit, reviewable line is worth
#      more than the operational convenience of Autopilot.
#   2. Autopilot rewrites Pod resource requests. The manifests size
#      geo-engine's connection pool against its replica ceiling, and
#      having the platform quietly adjust requests underneath that
#      arithmetic would make the capacity planning wrong in a way nobody
#      would notice until Postgres ran out of connections.
#
# Autopilot is a reasonable alternative if you would rather not manage
# node pools; if you switch, re-check both points above.

resource "google_container_cluster" "atlas" {
  name     = "${var.cluster_name}-${var.environment}"
  location = var.region

  network    = google_compute_network.atlas.id
  subnetwork = google_compute_subnetwork.nodes.id

  # The default node pool is created and immediately deleted so the
  # managed pool below is the only one. This is the documented way to get
  # a cluster with no default pool.
  remove_default_node_pool = true
  initial_node_count       = 1

  # THE reason this is a Standard cluster. ADVANCED_DATAPATH is Dataplane
  # V2 (eBPF/Cilium); it enforces NetworkPolicy natively, so the separate
  # network_policy addon must stay off — enabling both is rejected.
  datapath_provider = "ADVANCED_DATAPATH"

  networking_mode = "VPC_NATIVE"
  ip_allocation_policy {
    cluster_secondary_range_name  = "pods"
    services_secondary_range_name = "services"
  }

  private_cluster_config {
    enable_private_nodes = true
    # The control plane keeps a public endpoint, restricted by
    # master_authorized_networks below. Setting this true as well means
    # kubectl only works from inside the VPC, which needs a bastion or an
    # IAP tunnel — correct for a hardened environment, painful as a
    # default.
    enable_private_endpoint = false
    master_ipv4_cidr_block  = var.master_cidr
  }

  master_authorized_networks_config {
    dynamic "cidr_blocks" {
      for_each = var.authorized_networks
      content {
        cidr_block   = cidr_blocks.value.cidr_block
        display_name = cidr_blocks.value.display_name
      }
    }
  }

  # Workload Identity: Pods assume Google service accounts through their
  # Kubernetes service account, so no JSON key ever lands in a Secret.
  workload_identity_config {
    workload_pool = "${var.project_id}.svc.id.goog"
  }

  release_channel {
    channel = "REGULAR"
  }

  # Shielded nodes: secure boot and integrity monitoring.
  enable_shielded_nodes = true

  addons_config {
    http_load_balancing { disabled = false }
    horizontal_pod_autoscaling { disabled = false }
    # Off deliberately: Dataplane V2 provides policy enforcement, and GKE
    # rejects a cluster that asks for both.
    network_policy_config { disabled = true }
  }

  # Surface control-plane and workload logs/metrics in Cloud Operations.
  logging_config {
    enable_components = ["SYSTEM_COMPONENTS", "WORKLOADS"]
  }
  monitoring_config {
    enable_components = ["SYSTEM_COMPONENTS"]
    managed_prometheus {
      # Scrapes the prometheus.io/* annotations already on every Atlas
      # Deployment, so metrics work without running a Prometheus.
      enabled = true
    }
  }

  # Rolling upgrades on the REGULAR channel, but not in the middle of a
  # weekday afternoon.
  maintenance_policy {
    recurring_window {
      start_time = "2024-01-01T03:00:00Z"
      end_time   = "2024-01-01T07:00:00Z"
      recurrence = "FREQ=WEEKLY;BYDAY=SA,SU"
    }
  }

  deletion_protection = var.environment == "prod"

  depends_on = [google_service_networking_connection.private_service_connection]
}

resource "google_service_account" "nodes" {
  account_id   = "${var.cluster_name}-${var.environment}-nodes"
  display_name = "Atlas GKE nodes (${var.environment})"
}

# Least privilege for the node identity. Notably absent: any data-plane
# permission. Workloads get their own identities through Workload Identity
# (see iam.tf) rather than inheriting the node's.
resource "google_project_iam_member" "nodes" {
  for_each = toset([
    "roles/logging.logWriter",
    "roles/monitoring.metricWriter",
    "roles/monitoring.viewer",
    "roles/stackdriver.resourceMetadata.writer",
    "roles/artifactregistry.reader",
  ])
  project = var.project_id
  role    = each.value
  member  = "serviceAccount:${google_service_account.nodes.email}"
}

resource "google_container_node_pool" "general" {
  name     = "general"
  cluster  = google_container_cluster.atlas.id
  location = var.region

  # Per-zone count. The cluster is regional across three zones, so the
  # real node count is this multiplied by three.
  initial_node_count = var.node_min_count

  autoscaling {
    min_node_count = var.node_min_count
    max_node_count = var.node_max_count
  }

  management {
    auto_repair  = true
    auto_upgrade = true
  }

  upgrade_settings {
    # Add a node before taking one away, so an upgrade does not shrink
    # capacity while it runs.
    max_surge       = 1
    max_unavailable = 0
  }

  node_config {
    machine_type = var.node_machine_type
    disk_size_gb = 100
    disk_type    = "pd-balanced"

    service_account = google_service_account.nodes.email
    oauth_scopes    = ["https://www.googleapis.com/auth/cloud-platform"]

    workload_metadata_config {
      # Blocks Pods from reading the node's metadata server, which is what
      # stops a compromised container from stealing node credentials.
      mode = "GKE_METADATA"
    }

    shielded_instance_config {
      enable_secure_boot          = true
      enable_integrity_monitoring = true
    }

    labels = {
      environment = var.environment
      part-of     = "atlas"
    }

    tags = ["atlas", var.environment]
  }
}
