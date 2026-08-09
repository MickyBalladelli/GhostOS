# AGENTS.md

For repo-wide analysis, first run Bonsai. This is required for full project summaries, architecture review, onboarding, broad bug hunting, and any question that needs many files. Do not answer from ordinary file browsing until this command succeeds:

```sh
bonsai . --max-tokens 12000 --level 2 --output file --output-file /tmp/bonsai.xml
```

Then inspect `/tmp/bonsai.xml` before answering.

Expected behavior example:

```text
User asks: summarize this whole project
You run: bonsai . --max-tokens 12000 --level 2 --output file --output-file /tmp/bonsai.xml
You inspect: /tmp/bonsai.xml
Then answer from that context.
```
Do not touch the formatting of files you are not working on.
If you need to edit new temp files, use the temp folder. Clean up once done.
Do not guess, find the real root cause of an issue when you are trying to fix a bug.
Do not use a code formatter.