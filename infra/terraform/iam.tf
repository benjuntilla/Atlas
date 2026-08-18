# Workload Identity for the application Pods.
#
# One Google service account, mapped to the `atlas` Kubernetes service
# account in the `atlas` namespace. Pods using that KSA get GCP
# credentials with no JSON key anywhere — the node metadata path that
# would otherwise leak node identity is already blocked by GKE_METADATA in
# gke.tf.
#
# The KSA itself is a Kubernetes object, so it is created by the manifests
# rather than here; this binding is what makes it usable. The manifests
# currently run as `default`, so applying this is only useful once
# workloads are switched to the `atlas` KSA — which is the prerequisite
# for pulling secrets from Secret Manager instead of a static Secret.

locals {
  ksa_namespace = "atlas"
  ksa_name      = "atlas"
}

resource "google_service_account" "workload" {
  account_id   = "${var.cluster_name}-${var.environment}-workload"
  display_name = "Atlas workloads (${var.environment})"
}

resource "google_service_account_iam_member" "workload_identity" {
  service_account_id = google_service_account.workload.name
  role               = "roles/iam.workloadIdentityUser"
  member             = "serviceAccount:${var.project_id}.svc.id.goog[${local.ksa_namespace}/${local.ksa_name}]"
}

# Read-only on exactly the two secrets, granted per-secret rather than
# project-wide so this identity cannot read anything else added later.
resource "google_secret_manager_secret_iam_member" "database_url" {
  secret_id = google_secret_manager_secret.database_url.id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.workload.email}"
}

resource "google_secret_manager_secret_iam_member" "jwt_secret" {
  secret_id = google_secret_manager_secret.jwt_secret.id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.workload.email}"
}

# Lets workloads reach Cloud SQL through the auth proxy if they are ever
# switched to it. Harmless with direct private-IP connections, and the
# alternative is remembering to add it during an incident.
resource "google_project_iam_member" "workload_sql_client" {
  project = var.project_id
  role    = "roles/cloudsql.client"
  member  = "serviceAccount:${google_service_account.workload.email}"
}
