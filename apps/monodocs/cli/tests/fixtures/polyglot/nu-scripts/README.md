# nu-scripts

Fixture Nushell module.

## Usage

```nu
export def deploy [--namespace: string = "default"] {
  kubectl get pods --namespace $namespace | from json | get items
}
```
