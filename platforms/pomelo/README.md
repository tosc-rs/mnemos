## Running locally via dev server

```shell
$ trunk serve
```

## Static build for the web
```shell
# dependencies
$ cargo install --locked trunk
$ cargo install --locked wasm-bindgen-cli

# build to dist/, suitable for http(s)::/host
$ trunk build --release

# build to dist/ with a custom webroot dir, e.g. http(s)://host/my/webroot
$ trunk build --release --public-url='/my/webroot/'
```

## Licensing

This package contains a vendored copy of [Xterm.js](http://xtermjs.org/). Its license (MIT) is preserved in `src/js/xterm.css`.