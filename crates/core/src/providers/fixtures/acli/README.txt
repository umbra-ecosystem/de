SYNTHETIC VALUES, REAL STRUCTURE.

These files reproduce the structure that a real `acli 1.3.39-stable` prints (checked with
read-only commands on a real Jira and reduced to keys, node types and value kinds), filled
with invented data (Jane Doe, PROJ-123, acct-0001, example.atlassian.net). No real ticket
text, names, emails, account ids or hosts are here, and none may be added.

  search.json               `workitem search --json`: top-level array of issues, with the
                            many keys acli leaves `null`; no `updated` (search rejects it).
  view.json                 `workitem view KEY --json --fields ...,updated`: one issue.
  view_comments.json        `workitem view KEY --json --fields comment`: comments with author
                            account ids, ADF bodies (mention, emoji, inlineCard, media, table
                            and an invented future node type) and Jira timestamps.
  comment_list_flat.json    `workitem comment list --key K --json`: the flat shape with the
                            author as a display-name string, a plain body and no account id or
                            timestamp. The adapter must NOT use this command; kept to prove it.
