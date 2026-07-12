:i count 3
:b shell 59
python tests/recursive/driver.py smoke/fresh-journal-orient
:i returncode 0
:b stdout 122
scenario: smoke/fresh-journal-orient
scope_kind: journal
active_count: 0
closed_count: 0
hidden_count: 0
status: ok

:b stderr 0

:b shell 57
python tests/recursive/driver.py smoke/journal-vs-subtree
:i returncode 0
:b stdout 302
scenario: smoke/journal-vs-subtree
journal_scope_kind: journal
subtree_scope_kind: subtree
journal_member_slugs: child,grandchild,outsider,root
subtree_member_slugs: child,grandchild,root
outsider_in_journal: yes
outsider_in_subtree: no
child_in_journal: yes
child_in_subtree: yes
status: ok

:b stderr 0

:b shell 63
python tests/recursive/driver.py smoke/refs-do-not-expand-scope
:i returncode 0
:b stdout 113
scenario: smoke/refs-do-not-expand-scope
subtree_member_slugs: child,root
evidence_in_subtree: no
status: ok

:b stderr 0

