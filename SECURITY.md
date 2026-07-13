# Security policy

## Reporting a vulnerability

Do not open a public issue for an unpatched vulnerability. Use GitHub's private
vulnerability-reporting or Security Advisory flow for this repository. Include
the affected version, reproduction, impact, and any proposed mitigation.

## Supported versions

Until mnema reaches 1.0, security fixes are made on the latest released minor
version. Older development snapshots may require upgrading.

## Security boundaries

- A mnema journal is plaintext JSONL, not a secret store. Do not write API keys,
  credentials, private prompts, or regulated data into entries or provenance.
- Hash chains and anchors detect mutation; they do not encrypt content or prove
  that the original writer was trustworthy.
- Local multi-writer safety relies on filesystem locking. Separate machines do
  not share that lock and must not concurrently append to one journal copy.
- Refs, markers, and task lifecycle are semantic facts, not authorization or
  process-containment mechanisms.
- Exported journals contain the recorded plaintext and should be protected like
  the source journal.

When reporting an issue, distinguish journal integrity, confidentiality,
availability, and surrounding process-harness behavior so the responsible
boundary remains clear.
