# Flux KCL Operator

The Flux KCL Operator is a Kubernetes operator that enables declarative management of KCL configurations in a Flux-native way. It integrates with Flux's source controller to fetch KCL modules from Git repositories and OCI registries, then renders and applies them to your cluster.

## Features

- Manages KCL configurations as Kubernetes custom resources
- Integrates with Flux source controller (GitRepository and OCIRepository)
- Renders KCL modules with configurable arguments
- Handles dependencies through KCL's module system
- Manages the lifecycle of resources created from KCL configurations
- Automatic redeployment when source or configuration changes
- Built-in garbage collection of managed resources
- Events and status reporting

## Installation

1. Install the Custom Resource Definition (CRD):

```bash
# Generate the CRD YAML
flux-kcl-operator crd > kcl-instance-crd.yaml

# Apply it to your cluster
kubectl apply -f kcl-instance-crd.yaml
```

2. Deploy the operator:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: flux-kcl-operator
  namespace: flux-system
spec:
  selector:
    matchLabels:
      app: flux-kcl-operator
  template:
    metadata:
      labels:
        app: flux-kcl-operator
    spec:
      containers:
      - name: manager
        image: ghcr.io/kcl-lang/flux-kcl-operator:latest
        args:
        - run
        env:
        - name: RUST_LOG
          value: info
```

## Usage

1. First, create a GitRepository or OCIRepository source containing your KCL configuration:

```yaml
apiVersion: source.toolkit.fluxcd.io/v1
kind: GitRepository
metadata:
  name: my-kcl-config
  namespace: flux-system
spec:
  interval: 1m
  url: https://github.com/example/kcl-configs
  ref:
    branch: main
```

2. Create a KclInstance that references your source:

```yaml
apiVersion: kcl.evrone.com/v1alpha1
kind: KclInstance
metadata:
  name: my-app-config
  namespace: default
spec:
  sourceRef:
    name: my-kcl-config
    namespace: flux-system
    kind: GitRepository
  path: ./configs/my-app
  instanceConfig:
    arguments:
      env: production
      replicas: "3"
  interval: 5m
```

## Configuration

The `KclInstance` spec supports the following fields:

- `sourceRef`: Reference to a Flux source (GitRepository or OCIRepository)
- `path`: Path to the KCL module within the source
- `instanceConfig`: Configuration for KCL rendering
  - `arguments`: Key-value pairs passed as arguments to the KCL program
  - `vendor`: Enable vendoring of dependencies
  - `sortKeys`: Sort keys in output
  - `showHidden`: Show hidden attributes
- `interval`: Reconciliation interval

## Building

```bash
cargo build --release
```

## Development

Requirements:
- Rust toolchain
- Kubernetes cluster (local or remote)
- kubectl

Run locally:

```bash
cargo run -- run
```

Run tests:

```bash
cargo test
```

## Roadmap

- [x] Read argumets from reference objects (e.g. ConfigMap or Secret)
- [x] Render KCL modules
- [x] Handle KCL dependencies
- [x] Manage Rendered Kubernetes resources
- [x] Garbage collection on delete KCL instance
- [X] Handle configuration drift detection
- [X] Handle resources drift detection (remove old resources)
- [X] Get arguments from reference objects
- [ ] Handle pre/post-render hooks
- [ ] Handle versioning
- [ ] Handle rollbacks

## Future Work
- Refactor controller code to improve event firing at reconciliation
- Compute conditions for resources managed by KCL/KclInstances
- Add support for dependsOn
- Add support for configuration drift detection (and diffs for configuration)
- Add support for pre/post-render hooks with Kubernetes Jobs
- Add version support for KCL Instance deployments
- Add support for rollbacks

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
