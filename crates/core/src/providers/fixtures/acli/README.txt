FIXTURES DERIVED FROM DOCUMENTATION, NOT CAPTURED FROM A REAL INSTALL.

Atlassian's ACLI reference (https://developer.atlassian.com/cloud/acli/reference/commands/)
documents flags but not the JSON that `--json` prints. These files follow the Jira REST
shape and Atlassian Document Format (ADF) documentation. When a real `acli` is available,
run the probe (see providers/acli.rs) and replace these with the captured output.
