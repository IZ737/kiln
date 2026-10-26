# Security policy

Kiln compiles native executables and provides an HTTP/runtime stack, Glaze rendering, and Ember browser/RPC functionality. Security reports about the compiler, generated code, runtime, tooling, installers, or relevant dependencies are taken seriously.

## Supported versions

Kiln is **pre-1.0** and changes quickly. Security fixes are handled on a best-effort basis for the current development branch and, where practical, the latest published release. Older versions are **not guaranteed** security backports or ongoing support.

| Version | Security maintenance |
| --- | --- |
| `main` | Best-effort fixes |
| Latest release | Best-effort fixes when practical |
| Older releases | No guaranteed backports |

## Report a vulnerability privately

**Do not publish a working exploit, credentials, or sensitive vulnerability details in a public issue or pull request.**

1. Check the repository's [Security tab](https://github.com/IZ737/kiln/security) for a **Report a vulnerability** option. If private vulnerability reporting is enabled, use it.
2. If that option is unavailable, [open an issue](https://github.com/IZ737/kiln/issues/new/choose) containing **only a request for a private reporting channel**, without any technical exploit details, or contact the maintainer through an already-established private channel.
3. Provide the affected version/commit, environment, minimal reproduction, impact, relevant logs, and any suggested mitigation **only through the private channel**. Remove secrets and personal data.

The maintainer will investigate and coordinate a fix and disclosure where practical. Please allow time for analysis before making details public; no response or remediation deadline is promised.

For non-security bugs, use the normal [bug report form](https://github.com/IZ737/kiln/issues/new/choose). If you are unsure whether a bug is security-sensitive, use the private path first.
