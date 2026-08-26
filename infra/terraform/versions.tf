terraform {
  required_version = ">= 1.5"

  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 6.0"
    }
  }

  # State must be remote and locked before more than one person runs this.
  # Left commented rather than pointed at a bucket that does not exist, so
  # `terraform init` works out of the box for a first read-through; fill it
  # in and re-init before any shared use.
  #
  # backend "gcs" {
  #   bucket = "atlas-tfstate-<suffix>"
  #   prefix = "atlas/prod"
  # }
}

provider "google" {
  project = var.project_id
  region  = var.region
}
