# Governance

This repository is being prepared under the Pixel identity. The original RTO
maintainer list, project board, repository secrets, and Homebrew tap do not
establish who has access to `Pixel-CLI/pixel` or which destination resources
exist. Verify those details with the destination maintainers before enabling
repository automation that depends on them.

Until then, releases publish assets to this repository only. The release
workflow does not write to a separate Homebrew repository or project board.
The release includes a Homebrew formula as an asset, but no tap is configured.
