# kcl-config

Fixture KCL module.

## Usage

```kcl
schema Server:
    name: str = "web"
    replicas: int = 2

    check:
        replicas > 0, "replicas must be positive"
```
