# Feature flags (Flagsmith) for the todo product — imported by the root justfile.
# Reference: docs/feature-flags.md
#
#   just flags-bootstrap                     provision org/project/env + the six todo flags (idempotent)
#   just flags-list [identity]               print the effective flags — environment view, or for one user
#   just flags-set <flag> <on|off> [ident]   flip a flag for everyone, or override it for one user
#   just flags-open                          open the Flagsmith UI

flagsmith_url := env("FLAGSMITH_URL", "http://localhost:8000")

# Provision the local Flagsmith (org/project/environment + the six flags) and print the environment key
[group('flags')]
flags-bootstrap:
    bash scripts/just/flagsmith-bootstrap.sh

# Print the flags todo-api resolves: environment view, or `just flags-list demo-user` for one identity
[group('flags')]
flags-list identity="":
    bash scripts/just/flagsmith-flags.sh list {{ identity }}

# Flip a flag — `just flags-set todo_write off demo-user`; omit the identity to change the environment
[group('flags')]
flags-set flag state identity="":
    bash scripts/just/flagsmith-flags.sh set {{ flag }} {{ state }} {{ identity }}

# Open the Flagsmith UI (login shown by `just flags-bootstrap`)
[group('flags')]
flags-open:
    open {{ flagsmith_url }} 2>/dev/null || xdg-open {{ flagsmith_url }}
