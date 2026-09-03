# /// script
# requires-python = ">=3.10"
# dependencies = ["kubernetes"]
# ///
"""Print DevEnvironment status and, when ready, its connection details.

Usage: uv run example.py [claim-name] [namespace]
"""

import json
import re
import sys

import devenv


def main():
    name = sys.argv[1] if len(sys.argv) > 1 else "demo"
    namespace = sys.argv[2] if len(sys.argv) > 2 else "default"

    status = devenv.get(name, namespace)
    print(json.dumps(status, indent=2))

    if status["ready"]:
        conn = devenv.connection(name, namespace)
        if conn["postgres"] and conn["postgres"].get("password"):
            conn["postgres"]["password"] = "********"
            if conn["postgres"].get("uri"):
                conn["postgres"]["uri"] = re.sub(r"//([^:]+):[^@]+@", r"//\1:********@", conn["postgres"]["uri"])
        print(json.dumps(conn, indent=2))
    else:
        print(f"DevEnvironment {namespace}/{name} is not ready yet", file=sys.stderr)


if __name__ == "__main__":
    main()
