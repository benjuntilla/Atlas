variable "project_id" {
  description = "GCP project ID to deploy into."
  type        = string
}

variable "region" {
  description = "Primary region. Cloud SQL and the GKE control plane live here."
  type        = string
  default     = "us-central1"
}

variable "environment" {
  description = "Environment name, used in resource names and labels."
  type        = string
  default     = "prod"

  validation {
    condition     = contains(["dev", "staging", "prod"], var.environment)
    error_message = "environment must be one of: dev, staging, prod."
  }
}

variable "cluster_name" {
  description = "GKE cluster name."
  type        = string
  default     = "atlas"
}

# --- networking -------------------------------------------------------------

variable "subnet_cidr" {
  description = "Primary CIDR for the node subnet."
  type        = string
  default     = "10.0.0.0/20"
}

variable "pods_cidr" {
  description = "Secondary range for Pod IPs. Must be large: GKE allocates a /24 per node by default, so a /16 caps the cluster near 256 nodes."
  type        = string
  default     = "10.4.0.0/14"
}

variable "services_cidr" {
  description = "Secondary range for ClusterIP Services."
  type        = string
  default     = "10.8.0.0/20"
}

variable "master_cidr" {
  description = "RFC1918 /28 for the private control plane endpoint. Must not overlap anything else in the VPC."
  type        = string
  default     = "172.16.0.0/28"
}

variable "authorized_networks" {
  description = <<-DESC
    CIDRs allowed to reach the Kubernetes API. Defaults to empty, which
    means the API is reachable only from inside the VPC — kubectl then
    requires a bastion, Cloud Shell, or an IAP tunnel. Add your office or
    CI egress range to use kubectl directly.
  DESC
  type = list(object({
    cidr_block   = string
    display_name = string
  }))
  default = []
}

# --- node pool --------------------------------------------------------------

variable "node_machine_type" {
  description = "Machine type for the general node pool."
  type        = string
  default     = "e2-standard-4"
}

variable "node_min_count" {
  description = "Minimum nodes per zone. With 3 zones this is multiplied by 3."
  type        = number
  default     = 1
}

variable "node_max_count" {
  description = "Maximum nodes per zone."
  type        = number
  default     = 5
}

# --- database ---------------------------------------------------------------

variable "db_tier" {
  description = "Cloud SQL machine tier. db-custom-2-7680 is 2 vCPU / 7.5GB — a sane floor for PostGIS proximity queries."
  type        = string
  default     = "db-custom-2-7680"
}

variable "db_disk_size_gb" {
  description = "Initial disk size. Autoresize is on, so this is a floor rather than a cap."
  type        = number
  default     = 50
}

variable "db_deletion_protection" {
  description = "Block `terraform destroy` from deleting the database. Leave true anywhere with real data."
  type        = bool
  default     = true
}
