# Security policy

SkillsMate reads and changes files in your agents' skill folders and downloads skills from skills.sh, so security reports matter to us.

## Reporting a vulnerability

Report it privately through [GitHub's private vulnerability reporting](https://github.com/homielab/skillsmate/security/advisories/new). Please don't open a public issue.

Include what you found, how to reproduce it, and which version and operating system you used. We aim to reply within a week.

## In scope

- Paths that escape skill folders (symlinks, `..`, crafted names)
- Malicious SKILL.md files or skills.sh responses that crash the app, hang it, or show misleading text
- Changes made without a backup, or an Undo that loses data
- Data sent over the network beyond what the README describes

Security issues in skills themselves belong to their authors, or to skills.sh's audit partners.

## Supported versions

Only the latest release gets fixes.
