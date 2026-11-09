# worki's default agent image for Docker and Podman: small Alpine base, git tooling, Claude Code.
# Claude Code's native build supports musl; it needs libgcc, libstdc++ and a system ripgrep.
FROM docker.io/library/alpine:3.22

RUN apk add --no-cache bash git curl ca-certificates openssh-client libgcc libstdc++ ripgrep github-cli less jq

# Claude Code is fetched with curl and verified against the release manifest, the same steps
# install.sh takes. Its final `claude install` step re-downloads with Claude's own client, which
# fails inside Docker Desktop's network, so the verified binary is placed directly instead.
ARG CLAUDE_RELEASES=https://downloads.claude.ai/claude-code-releases
RUN set -eu; \
    case "$(uname -m)" in x86_64) arch=x64 ;; aarch64) arch=arm64 ;; *) echo "unsupported arch" >&2; exit 1 ;; esac; \
    platform="linux-$arch-musl"; \
    version=$(curl -fsSL --retry 5 "$CLAUDE_RELEASES/latest"); \
    checksum=$(curl -fsSL --retry 5 "$CLAUDE_RELEASES/$version/manifest.json" | jq -r ".platforms[\"$platform\"].checksum"); \
    curl -fsSL --retry 5 -o /usr/local/bin/claude "$CLAUDE_RELEASES/$version/$platform/claude"; \
    echo "$checksum  /usr/local/bin/claude" | sha256sum -c -; \
    chmod +x /usr/local/bin/claude; \
    claude --version

# Updates come from rebuilding the image; agents installed later go to ~/.local/bin.
ENV USE_BUILTIN_RIPGREP=0 DISABLE_AUTOUPDATER=1
RUN printf 'export PATH="$HOME/.local/bin:$PATH"\nexport USE_BUILTIN_RIPGREP=0\nexport DISABLE_AUTOUPDATER=1\n' > /etc/profile.d/worki.sh
RUN git config --global --add safe.directory '*'
