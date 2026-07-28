# Vendored HAP reference source

`hap.c`, `hap.h`, and `LICENSE` are copied without modification from
<https://github.com/Vidvox/hap> at commit:

```text
d847f6bbd3be88575dd4ef33a877243780e3be76
```

SHA-256:

```text
605e82917e492ed8eef02b3d8a8a47d8238c872e670b21ac3caf68c209d3eaac  hap.c
07ed233c327723f07773bada4c4dd5305fc86ae0fcb2f76a9c36fd97b9e0c6f2  hap.h
822f43b7dadf9e1b125af8ba42fad6d85e3f5d2f0c01f5428fb4fd143a699697  LICENSE
```

The C source uses the Snappy C ABI. `oneiroi-hap-sys` provides that ABI via
the pure-Rust `snap` crate instead of requiring a system C++ library.
