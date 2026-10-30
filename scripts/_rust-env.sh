# Sourced by the other scripts. Makes sure cargo is on the PATH: if it isn't,
# uses Homebrew's rustup, which isn't on the PATH by default. Otherwise
# explains how to install Rust.

if ! command -v cargo > /dev/null 2>&1; then
  for dir in /opt/homebrew/opt/rustup/bin /usr/local/opt/rustup/bin "$HOME/.cargo/bin"; do
    if [[ -x "$dir/cargo" ]]; then
      export PATH="$dir:$PATH"
      break
    fi
  done
fi

if ! command -v cargo > /dev/null 2>&1; then
  echo "Error: cargo not found. Install Rust with rustup: 'brew install rustup'" \
    "on macOS, or see https://rustup.rs. See README.md, Development." >&2
  exit 1
fi
