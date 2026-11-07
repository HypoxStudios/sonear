# Releasing

Sonear updates itself from GitHub, and only installs a build signed with the maker's key. A release is not ready until its setup file has a signature.

## Each release

1. Bump `version` in the root `Cargo.toml` and commit it.
2. Push the tag: `git tag v0.2.0` then `git push origin v0.2.0`. CI checks the tag against the version, runs the tests and makes a draft release with `sonear-0.2.0-setup.exe`.
3. Sign it: `.\scripts\sign-release.ps1 -Tag v0.2.0`. It downloads the file from the draft, asks for the key's password, checks the signature and uploads `sonear-0.2.0-setup.exe.minisig`. Pass `-KeyFile` to use a key file instead of Bitwarden.
4. Look over the draft and publish it.

Installed copies find it within about six hours, download it and show **Restart to update**. They never restart on their own.

## One-time setup

1. `minisign -G -p scripts/sonear-update.pub -s sonear.key` makes the key pair. Use a strong password.
2. Save the secret key in a Bitwarden secure note called `Sonear update signing key`, then delete `sonear.key`.
3. Put the second line of `scripts/sonear-update.pub` in `PUBLIC_KEY` in `crates/core/src/update.rs`. Until it is set, Sonear never checks for updates.

Losing the key means shipping a new public key in a release by hand, so keep a backup. A leaked key means the same.
