# Secret Manager containers for the two runtime secrets.
#
# Only the containers are created here — no versions. A secret VALUE set
# through Terraform is stored in state in plaintext, so the values are
# added out of band with `gcloud secrets versions add` (see the README).
# That is the difference between "the secret exists in one managed place"
# and "the secret is in the state file that CI can read".

resource "google_secret_manager_secret" "database_url" {
  secret_id = "atlas-database-url"

  replication {
    auto {}
  }

  labels = {
    environment = var.environment
    part-of     = "atlas"
  }
}

resource "google_secret_manager_secret" "jwt_secret" {
  secret_id = "atlas-jwt-secret"

  replication {
    auto {}
  }

  labels = {
    environment = var.environment
    part-of     = "atlas"
  }
}
