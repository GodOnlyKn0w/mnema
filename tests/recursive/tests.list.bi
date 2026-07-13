:i count 6
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

:b shell 56
python tests/recursive/driver.py full/depth-chain-orient
:i returncode 0
:b stdout 362
scenario: full/depth-chain-orient
chain_depth: 10
journal_member_slugs: depth-0,depth-1,depth-10,depth-2,depth-3,depth-4,depth-5,depth-6,depth-7,depth-8,depth-9
mid_depth5_scope_kind: subtree
mid_depth5_member_slugs: depth-10,depth-5,depth-6,depth-7,depth-8,depth-9
depth0_in_mid: no
depth4_in_mid: no
depth5_in_mid: yes
depth10_in_mid: yes
status: ok

:b stderr 0

:b shell 57
python tests/recursive/driver.py full/reparent-join-leave
:i returncode 0
:b stdout 209
scenario: full/reparent-join-leave
before_slugs: child,root
after_join_slugs: child,joiner,root
after_leave_slugs: joiner,root
joiner_before: no
joiner_after_join: yes
child_after_leave: no
status: ok

:b stderr 0

:b shell 69
python tests/recursive/driver.py crash/complete-batch-strict-readable
:i returncode 0
:b stdout 135
scenario: crash/complete-batch-strict-readable
doctor_exit: 0
subtree_member_slugs: child,root
evidence_in_subtree: no
status: ok

:b stderr 0

