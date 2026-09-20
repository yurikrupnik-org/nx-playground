# kcl-broken

Fixture for the lint rules. References that resolve — [usage](#usage), [the guide](docs/guide.md),
[an anchor](docs/guide.md#install) and [the site](https://kcl-lang.io) — must stay silent.

## Usage

Everything from here down is wrong on purpose.
[a file that is not there](./missing.md)

![an image that is not there](./missing.png)

[an anchor the guide does not have](docs/guide.md#nowhere)

Anchors into this document: the next one matches no heading here.

[no such section](#absent)

```
kcl run main.k
```

```brainfuck
++++[>++++<-]>.
```

* a `*` bullet with trailing whitespace: `monodocs fmt` rewrites both
