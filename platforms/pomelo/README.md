## running locally via dev server
```shell
# aliased via .cargo/config.toml
$ cargo run-wasm-sim
```

or, if you have `trunk` installed (pro: automatic recompile + reload)

```shell
$ trunk serve
```

## static build for the web
```shell
# dependencies
$ cargo install --locked trunk
$ cargo install --locked wasm-bindgen-cli

# build to dist/, suitable for http(s)::/host
$ trunk build

# build to dist/ with a custom webroot dir, e.g. http(s)://host/my/webroot
$ trunk build --public-url='/my/webroot/'
```