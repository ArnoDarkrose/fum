<h3 align="center">
  <img src="https://raw.githubusercontent.com/ArnoDarkrose/fum/refs/heads/main/repo/logo.png" width="200"/>
</h3>

<h2 align="center">
  fum: A fully ricable tui-based mpris youtube music client.
</h2>

<p align="center">
  fum is a tui-based mpris music client designed to provide a simple and efficient way to display and control your youtube music within a tui interface.
</p>

<p align="center">
  <a href="https://github.com/ArnoDarkrose/fum/blob/main/LICENSE">
    <img src="https://img.shields.io/badge/MIT-DEFEDF?style=for-the-badge&logo=Pinboard&label=License&labelColor=1C1B22" />
  </a>

  <a href="https://github.com/ArnoDarkrose/fum/stargazers">
    <img src="https://img.shields.io/github/stars/ArnoDarkrose/fum?style=for-the-badge&logo=Apache%20Spark&logoColor=ffffff&labelColor=1C1B22&color=DEFEDF" />
  </a>
</p>
This is a fork of the original fum. This repo contains additions oriented on working specifically with youtube music. For example, in contrast to the original fum, this version has the ability to like and dislike currently playing video

## Demo

<img
  width="800px"
  src="https://github.com/user-attachments/assets/930283d8-6299-4ef9-865b-26960dcee866"
/>

## Installation

### Releases
Navigate to the releases page and download the latest binary

### Cargo (From Source)

```bash
git clone https://github.com/ArnoDarkrose/fum.git
cd fum
cargo build --release

# Either copy/move `target/release/yum` to /usr/bin
# Or add the release path to your system's path

# Moving fum binary to /usr/bin
mv target/release/fum /usr/bin
```

### Getting started
Before using fum, you will need to authorize to your google account, which can be done by passing `--authorize` key to the app. After that, you can use it normally, just like original fum.

### Configuring

See [Wiki](https://github.com/ArnoDarkrose/fum/wiki/Configuring)

### Need help?

Text me on Telegram @ArnoDarkrose

## Showcase on a rice

<img src="https://github.com/ArnoDarkrose/fum/blob/main/repo/showcase.png" />

## LICENSE

[MIT](https://github.com/ArnoDarkrose/fum/blob/main/LICENSE)
