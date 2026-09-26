#!/usr/bin/env nu
# Static check of every Nushell script in the repo: the body of `task lint-nu`
# (scripts/tasks/nu.yml).
#
# `nu --ide-check` parses and type-checks a file WITHOUT running it and prints
# one JSON record per line: `diagnostic` records carry a severity and a byte
# span into the file, `hint` records are inferred types (ignored here). It
# exits 0 even when it reports errors, so the verdict is computed here.
#
# `--no-config-file` on the child for the same reason every task passes it:
# a command or alias defined in someone's config.nu must not make a script
# pass locally that fails in CI.
#
# Unknown bare words are not errors: nu treats them as external commands and
# only fails at run time, so a missing binary is caught by running the task,
# not by this gate.

# Check the given .nu files, or every tracked + untracked-not-ignored .nu file.
def main [...files: path] {
  let files = if ($files | is-empty) {
    ^git ls-files --cached --others --exclude-standard -- '*.nu' | lines
  } else {
    $files
  }
  if ($files | is-empty) {
    print 'lint-nu: no .nu files'
    return
  }

  let diagnostics = $files | each {|file| check $file } | flatten
  for d in $diagnostics {
    print $"($d.file):($d.line):($d.column): ($d.severity): ($d.message)"
  }

  let errors = $diagnostics | where severity == 'Error' | length
  print $"lint-nu: ($files | length) files, ($errors) errors, ($diagnostics | length) diagnostics"
  if $errors > 0 { exit 1 }
}

# Diagnostics for one file, with the byte span resolved to line:column.
def check [file: path] {
  let src = open --raw $file | into binary
  ^$nu.current-exe --no-config-file --ide-check 100 $file
  | lines
  | each {|line| $line | from json }
  | where type == 'diagnostic'
  | each {|d|
    let before = if $d.span.start > 0 {
      $src | bytes at ..<($d.span.start) | decode utf-8
    } else {
      ''
    }
    let lines = $before | split row "\n"
    {
      file: $file
      line: ($lines | length)
      column: (($lines | last | str length --grapheme-clusters) + 1)
      severity: $d.severity
      message: $d.message
    }
  }
}
