## running locally via dev server
```shell
# see .cargo/config.toml
$ cargo run-wasm-sim
```

or, if you have `trunk` installed,

```shell
trunk serve
```

## building for the web
```shell
# dependencies
$ cargo install --locked trunk
$ cargo install --locked wasm-bindgen-cli

# build to dist/
$ trunk build

# build to dist/ with a custom webroot dir
$ trunk build --public-url='/my/webroot/'
```