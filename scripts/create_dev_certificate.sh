#!/bin/sh
# Creates "worki dev", a self-signed code-signing certificate, in your login Keychain. Run it once:
# start_dev.sh then signs dev builds with it, so macOS keeps "Always Allow" for worki's Keychain
# item across rebuilds.
set -eu

name="worki dev"
keychain=${KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}
if security find-certificate -c "$name" "$keychain" >/dev/null 2>&1; then
  echo "\"$name\" is already in your Keychain."
  exit 0
fi

dir=$(mktemp -d "${TMPDIR:-/tmp}/worki-cert.XXXXXX")
trap 'rm -rf "$dir"' EXIT
pass=$(/usr/bin/openssl rand -hex 16)
# The system's LibreSSL writes a PKCS#12 file `security import` can read; OpenSSL 3 doesn't by default.
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=$name" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning" \
  -addext "basicConstraints=critical,CA:false" \
  -keyout "$dir/key.pem" -out "$dir/cert.pem" 2>/dev/null
/usr/bin/openssl pkcs12 -export -name "$name" -inkey "$dir/key.pem" -in "$dir/cert.pem" \
  -passout "pass:$pass" -out "$dir/dev.p12"
security import "$dir/dev.p12" -k "$keychain" -P "$pass" -T /usr/bin/codesign >/dev/null

echo "Created \"$name\". The next ./start_dev.sh signs worki with it. Allow the Keychain prompt"
echo "once more with \"Always Allow\"; after that it stays quiet across rebuilds."
