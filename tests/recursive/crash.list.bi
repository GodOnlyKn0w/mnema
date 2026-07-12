:i count 1
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

