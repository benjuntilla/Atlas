# Kubernetes manifests

Plain YAML, applied with `kubectl apply -k`. Kustomize is used only for the
base/overlay split — there are no templating engines and no Helm chart,
because a platform this size does not have enough variance to justify one.

## Layout

```
infra/k8s/
├── base/                 # Everything, sized for a real cluster
│   ├── kustomization.yaml
│   ├── namespace.yaml
│   ├── config.yaml       # Non-secret env shared by every service
│   ├── secrets.example.yaml   # Template — NOT applied, see below
│   ├── migrate-job.yaml  # Runs before any rollout
│   ├── auth-service.yaml
│   ├── geo-engine.yaml
│   ├── payments-service.yaml
│   ├── gateway.yaml      # The only public ingress
│   ├── control-plane.yaml
│   ├── consumers.yaml    # All three background workers
│   ├── autoscaling.yaml  # HPAs + PodDisruptionBudgets
│   └── network-policy.yaml
├── overlays/
│   ├── dev/              # 1 replica, no HPA, relaxed resources
│   └── prod/             # 3 replicas, HPAs on, topology spread
└── kafka-topics.yaml     # Strimzi KafkaTopic CRDs (applied separately)
```

## Apply

```bash
kubectl apply -k infra/k8s/overlays/prod
```

## Secrets are not in this repo

`secrets.example.yaml` documents the required keys with placeholder values
and is excluded from the kustomization, so `kubectl apply -k` will never
create it. Supply the real values through your secret manager — on GKE
that is Secret Manager surfaced via the Secrets Store CSI driver, or
External Secrets Operator. The required keys are:

| Key | Used by | Notes |
|---|---|---|
| `database-url` | every service that touches Postgres | Full DSN including credentials |
| `jwt-secret` | auth-service | Must be ≥ 32 bytes; rotating it invalidates every live token |

## Ordering

The migration Job is a Kustomize resource with a `pre-install`-style
ordering enforced the only way plain Kubernetes offers: every Deployment
that touches Postgres runs an init container that blocks until the Job
reports success. Kubernetes has no native "wait for Job" dependency for
Deployments, and an init container is more honest than a `sleep`.

Services never run migrations themselves. With three replicas rolling
simultaneously that would be a race, and a failed migration would crash
every pod instead of failing one Job and leaving the previous version
serving.
