import manifests

import .apps
import .resources

_filter = option("app", default="")
_ns_filter = option("namespace", default="")

_selected = [a for a in apps.apps if (not _filter or a.name == _filter) and (not _ns_filter or a.namespace == _ns_filter)]

_all_resources = lambda app_list: [any] -> [any] {
    [r for app in app_list for r in [
        resources.make_service_account(app)
        resources.make_deployment(app)
        if app.enable_service:
            resources.make_service(app)
        if app.hpa.enabled:
            resources.make_hpa(app)
    ] if r]
}

items = _all_resources(_selected)

manifests.yaml_stream(items)
