"""
Generates Tiltfile content for each app.

Usage:
  # Print all Tiltfiles as YAML (for review)
  kcl run k8s-resource/gen_tiltfiles.k

  # Print a single app's Tiltfile (use namespace-name key)
  kcl run k8s-resource/gen_tiltfiles.k -D app=zerg-api
"""
import .apps
import .tiltfile as tf
import .schema

_filter = option("app", default="")

_selected = {
    k: v for k, v in tf.tiltfiles if not _filter or k == _filter
}

tiltfiles = _selected
