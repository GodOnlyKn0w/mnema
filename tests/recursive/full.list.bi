:i count 2
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

