# rust-app

Fixture binary.

## Usage

```rust
#[derive(Debug)]
struct Config { name: String }

fn main() {
    println!("{:?}", Config { name: "x".into() });
}
```
