# Security Policy

## Secret Material

Never commit real private keys, signing keys, API tokens, or environment files.

Install the repository hooks before committing or pushing:

```bash
just install-git-hooks
```

The hooks run the EVM private key scanner before commits and pushes. The pre-push
hook scans the commit objects that would be sent to the remote, so a secret that
was added and later removed in the same branch is still blocked before it reaches
GitHub.

You can run the scanner manually:

```bash
just check-secrets
just check-secrets --full-derive
```

## Reporting a Vulnerability

Do not open public GitHub issues for suspected security vulnerabilities.

Please report vulnerabilities privately through GitHub Security Advisories for this repository. If
you cannot use that workflow, contact the Taiko maintainers through an established private channel
before any public disclosure.

Include, when possible:

- affected commit or release
- impact summary
- reproduction steps or proof of concept
- any suggested mitigation

We will evaluate reports privately and coordinate disclosure once a fix or mitigation is available.
