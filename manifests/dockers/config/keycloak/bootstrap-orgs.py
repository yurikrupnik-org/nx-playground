#!/usr/bin/env python3
"""Idempotent local-dev bootstrap for the terran Keycloak organization.

Realm import enables Organizations (`organizationsEnabled`) and creates the
`organization` client scope, but it does NOT carry organizations/members — so this
script (run after `just reset-db` / `just _docker-up`) creates the demo organization,
adds the seeded test user, and grants the `terran-api` client the `organization`
scope so access tokens carry the org claim.

Pure stdlib (urllib); override defaults via env (KC_URL, KC_ADMIN, KC_ADMIN_PASSWORD,
REALM, CLIENT_ID, ORG_ALIAS, ORG_NAME, ORG_DOMAIN, MEMBER_USERNAME).
"""

import json
import os
import urllib.error
import urllib.parse
import urllib.request

KC = os.environ.get("KC_URL", "http://localhost:8088")
ADMIN = os.environ.get("KC_ADMIN", "admin")
ADMIN_PW = os.environ.get("KC_ADMIN_PASSWORD", "admin")
REALM = os.environ.get("REALM", "terran")
CLIENT_ID = os.environ.get("CLIENT_ID", "terran-api")
ORG_ALIAS = os.environ.get("ORG_ALIAS", "demo-corp")
ORG_NAME = os.environ.get("ORG_NAME", "Demo Corp")
ORG_DOMAIN = os.environ.get("ORG_DOMAIN", "terran.dev")
MEMBER = os.environ.get("MEMBER_USERNAME", "test@terran.dev")


def req(method, path, token=None, body=None, form=None):
    url = f"{KC}{path}"
    headers = {}
    data = None
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if form is not None:
        data = urllib.parse.urlencode(form).encode()
        headers["Content-Type"] = "application/x-www-form-urlencoded"
    elif body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    r = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(r) as resp:
            raw = resp.read().decode()
            return resp.status, (json.loads(raw) if raw else None)
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def main():
    status, tok = req(
        "POST",
        f"/realms/master/protocol/openid-connect/token",
        form={"grant_type": "password", "client_id": "admin-cli", "username": ADMIN, "password": ADMIN_PW},
    )
    if status != 200:
        raise SystemExit(f"admin login failed ({status}): {tok}")
    token = tok["access_token"]

    # 1. Ensure organizations are enabled on the realm.
    _, realm = req("GET", f"/admin/realms/{REALM}", token=token)
    if not realm.get("organizationsEnabled"):
        realm["organizationsEnabled"] = True
        req("PUT", f"/admin/realms/{REALM}", token=token, body=realm)
        print("enabled organizations on realm")

    # 2. Create the organization (ignore if it already exists).
    s, _ = req(
        "POST",
        f"/admin/realms/{REALM}/organizations",
        token=token,
        body={"name": ORG_NAME, "alias": ORG_ALIAS, "domains": [{"name": ORG_DOMAIN, "verified": True}]},
    )
    print(f"create org {ORG_ALIAS}: {s}")
    _, orgs = req("GET", f"/admin/realms/{REALM}/organizations", token=token)
    org = next((o for o in orgs if o["alias"] == ORG_ALIAS), None)
    if not org:
        raise SystemExit("organization not found after create")

    # 3. Add the member (ignore if already a member).
    s, uobj = req("GET", f"/admin/realms/{REALM}/users?username={urllib.parse.quote(MEMBER)}", token=token)
    if uobj:
        uid = uobj[0]["id"]
        s, _ = req("POST", f"/admin/realms/{REALM}/organizations/{org['id']}/members", token=token, body=uid)
        print(f"add member {MEMBER}: {s}")
    else:
        print(f"member {MEMBER} not found (seed not applied?) — skipping")

    # 4. Grant terran-api the `organization` client scope (so it can request the claim).
    _, scopes = req("GET", f"/admin/realms/{REALM}/client-scopes", token=token)
    org_scope = next((sc for sc in scopes if sc["name"] == "organization"), None)
    _, clients = req("GET", f"/admin/realms/{REALM}/clients?clientId={CLIENT_ID}", token=token)
    if org_scope and clients:
        s, _ = req(
            "PUT",
            f"/admin/realms/{REALM}/clients/{clients[0]['id']}/default-client-scopes/{org_scope['id']}",
            token=token,
        )
        print(f"grant org scope to {CLIENT_ID}: {s}")

    print("terran org bootstrap complete")


if __name__ == "__main__":
    main()
