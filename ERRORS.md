[Release commit message files]
What didn't work: Commit messages stored under /tmp disappeared while the
pre-commit hook ran, so Git could not read them after the checks passed.
What worked: Store the message under .git and pass that path to git commit -F.
Note for next time: Keep transient commit messages inside the repository's Git
metadata when the pre-commit hook invokes npm tooling.
