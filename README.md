![cargo-depot](logo.jpg)

This [cargo subcommand](https://doc.rust-lang.org/book/ch14-05-extending-cargo.html) allows creating and maintaining a simple [cargo alternative registry](https://doc.rust-lang.org/cargo/reference/registries.html), which can be served using any webserver (nginx, caddy, miniserve, S3...).

Crates are added to the registry by pointing to their source, as a local folder or remote tarball, via the `cargo depot` subcommand.

### Handling of git and path dependencies

A distinguishing feature compared to other tools (see [below](#see-also)) is that [git and path dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html) in the added crates will refer to the registry rather than crates.io, with the understanding that these will be pushed to the registry as well. This removes the need for maintaining [forks to replace dependencies](https://www.reddit.com/r/rust/comments/15z3w34/parch_indirect_dependencies/) or complex `[patch]` or `[source]` cargo configurations.

For example, a crate with

```toml
[dependencies]
git_dep = { version = "0.1.0", git = "https://github.com/cpg314/example.git", tag = "v0.1.0" }
path_dep = { version = "0.1.0", path = "../example" }
```

will have these dependencies advertised in the index as being in the local registry. With `cargo publish`, they would refer to the `crates.io` registry unless a `registry` property is added.

> [!NOTE]  
> When creating a tarball, `cargo package` will try to [create a lockfile](https://github.com/rust-lang/cargo/blob/ec05ed9f9cf03f516f469611d3fde7573300d187/src/cargo/ops/cargo_package.rs#L967) for crates [containing binaries or examples](https://github.com/rust-lang/cargo/blob/ec05ed9f9cf03f516f469611d3fde7573300d187/src/cargo/core/package.rs#L187-L189). This is problematic, as the registry dependencies will not resolve, unless again a `registry` property is added and the dependencies are pushed in the right order. To avoid this, `cargo-depot` will automatically temporarily disable binary targets and examples.

### Non-features

On the other hand, this is _not_ a fully-fledged registry; for example, the [web API](https://doc.rust-lang.org/cargo/reference/registry-web-api.html) is not implemented.

## Usage

### Installation

```
$ cargo install --git https://github.com/cpg314/cargo-local-registry
```

### Initializing and maintaining the registry

```
Usage: cargo local-registry [OPTIONS] --registry <REGISTRY> [CRATES]...

Arguments:
  [CRATES]...  Paths to crates (local workspaces or HTTP links to tar.gz)

Options:
      --registry <REGISTRY>  Local path to the registry
      --url <URL>            URL of the registry, only needed for initialization
  -h, --help                 Print help
  -V, --version              Print version
```

Versions that have already been added are skipped.

On Github, tarballs can be downloaded at given commits or tags with the following links:

```text
https://github.com/{owner}/{repo}/archive/{commit}.tar.gz
https://github.com/{owner}/{repo}/archive/refs/tags/{tag}.tar.gz
```

### Serving the files

Use your favourite HTTP server to serve the contents of the registry folder (`crates` and `index` folders).

### Using the registry

In your [`.cargo/config.toml`](https://doc.rust-lang.org/cargo/reference/config.html#hierarchical-structure):

```toml
[registries]
local = { index = "sparse+http://127.0.0.1:3333/index/" }
```

(replace the URL adequately).

Finally, when declaring your dependencies in `Cargo.toml`:

```toml
crate = {version = "0.1.1", registry = "local" }
```

### Deleting a crate

Delete the line in the index file in the `index` directory (or the entire file to delete all versions), and the `.crate` file in the `.crate` directory. This might break things for users.

## Test

The following will create a registry, add crates to it, and finally access them in a crate:

```
$ cargo make example
```

See the files in the `example` directory.

## See also

- <https://github.com/integer32llc/margo>
- <https://github.com/ehuss/cargo-index/> (an earlier implementation used the corresponding library [reg-index](https://github.com/ehuss/cargo-index/tree/master/reg-index), but this introduces a fairly heavy dependency on `git2` and therefore `openssl`).
- https://github.com/rust-lang/cargo/wiki/Third-party-registries
