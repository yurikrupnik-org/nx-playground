# ts-lib

Fixture package that links to [rust-app](../rust-app/README.md).

## Usage

```ts
export async function render(name: string): Promise<string> {
  return `hello ${name}`;
}
```
