# VPC for the platform.
#
# Nodes are private (no external IPs), so egress — pulling images from
# GHCR, reaching Let's Encrypt — goes through Cloud NAT. Without the NAT
# a private cluster comes up and then fails every image pull, which is a
# confusing way to discover the dependency.

resource "google_compute_network" "atlas" {
  name                    = "${var.cluster_name}-${var.environment}"
  auto_create_subnetworks = false
  # Regional routing keeps cross-zone traffic inside the region's fabric.
  routing_mode = "REGIONAL"
}

resource "google_compute_subnetwork" "nodes" {
  name          = "${var.cluster_name}-${var.environment}-nodes"
  network       = google_compute_network.atlas.id
  region        = var.region
  ip_cidr_range = var.subnet_cidr

  # VPC-native cluster: Pods and Services get real routable ranges rather
  # than an overlay. Required for Dataplane V2 and for private Cloud SQL.
  secondary_ip_range {
    range_name    = "pods"
    ip_cidr_range = var.pods_cidr
  }
  secondary_ip_range {
    range_name    = "services"
    ip_cidr_range = var.services_cidr
  }

  # Flow logs are the only way to answer "did the NetworkPolicy actually
  # drop that" after the fact. Sampled at 50% to bound the log bill.
  log_config {
    aggregation_interval = "INTERVAL_10_MIN"
    flow_sampling        = 0.5
    metadata             = "INCLUDE_ALL_METADATA"
  }

  private_ip_google_access = true
}

resource "google_compute_router" "nat" {
  name    = "${var.cluster_name}-${var.environment}-router"
  network = google_compute_network.atlas.id
  region  = var.region
}

resource "google_compute_router_nat" "nat" {
  name                               = "${var.cluster_name}-${var.environment}-nat"
  router                             = google_compute_router.nat.name
  region                             = var.region
  nat_ip_allocate_option             = "AUTO_ONLY"
  source_subnetwork_ip_ranges_to_nat = "ALL_SUBNETWORKS_ALL_IP_RANGES"

  log_config {
    enable = true
    filter = "ERRORS_ONLY"
  }
}

# --- private services access for Cloud SQL ----------------------------------
#
# Cloud SQL with a private IP lives in a Google-managed VPC peered to this
# one. The address range below is reserved for that peering; without it,
# creating the instance fails with a peering error that does not obviously
# point back here.

resource "google_compute_global_address" "private_service_range" {
  name          = "${var.cluster_name}-${var.environment}-sql-range"
  purpose       = "VPC_PEERING"
  address_type  = "INTERNAL"
  prefix_length = 16
  network       = google_compute_network.atlas.id
}

resource "google_service_networking_connection" "private_service_connection" {
  network                 = google_compute_network.atlas.id
  service                 = "servicenetworking.googleapis.com"
  reserved_peering_ranges = [google_compute_global_address.private_service_range.name]
}
