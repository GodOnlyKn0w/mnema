:i count 6
:b shell 59
python tests/recursive/driver.py smoke/fresh-journal-orient
:i returncode 1
:b stdout 0

:b stderr 1381
Exception in thread Thread-3 (_readerthread):
Traceback (most recent call last):
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1075, in _bootstrap_inner
    self.run()
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1012, in run
    self._target(*self._args, **self._kwargs)
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\subprocess.py", line 1601, in _readerthread
    buffer.append(fh.read())
                  ^^^^^^^^^
UnicodeDecodeError: 'gbk' codec can't decode byte 0xb3 in position 264: illegal multibyte sequence
Traceback (most recent call last):
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 378, in <module>
    main(sys.argv[1:])
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 372, in main
    SCENARIOS[name](project)
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 186, in scenario_smoke_fresh_journal
    orient = p.orient()
             ^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 146, in orient
    return self.run_json(args)
           ^^^^^^^^^^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 98, in run_json
    text = proc.stdout.strip()
           ^^^^^^^^^^^^^^^^^
AttributeError: 'NoneType' object has no attribute 'strip'

:b shell 57
python tests/recursive/driver.py smoke/journal-vs-subtree
:i returncode 1
:b stdout 0

:b stderr 1390
Exception in thread Thread-11 (_readerthread):
Traceback (most recent call last):
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1075, in _bootstrap_inner
    self.run()
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1012, in run
    self._target(*self._args, **self._kwargs)
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\subprocess.py", line 1601, in _readerthread
    buffer.append(fh.read())
                  ^^^^^^^^^
UnicodeDecodeError: 'gbk' codec can't decode byte 0xb3 in position 1529: illegal multibyte sequence
Traceback (most recent call last):
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 378, in <module>
    main(sys.argv[1:])
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 372, in main
    SCENARIOS[name](project)
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 205, in scenario_smoke_journal_vs_subtree
    journal = p.orient()
              ^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 146, in orient
    return self.run_json(args)
           ^^^^^^^^^^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 98, in run_json
    text = proc.stdout.strip()
           ^^^^^^^^^^^^^^^^^
AttributeError: 'NoneType' object has no attribute 'strip'

:b shell 63
python tests/recursive/driver.py smoke/refs-do-not-expand-scope
:i returncode 1
:b stdout 0

:b stderr 1409
Exception in thread Thread-11 (_readerthread):
Traceback (most recent call last):
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1075, in _bootstrap_inner
    self.run()
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1012, in run
    self._target(*self._args, **self._kwargs)
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\subprocess.py", line 1601, in _readerthread
    buffer.append(fh.read())
                  ^^^^^^^^^
UnicodeDecodeError: 'gbk' codec can't decode byte 0xb3 in position 940: illegal multibyte sequence
Traceback (most recent call last):
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 378, in <module>
    main(sys.argv[1:])
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 372, in main
    SCENARIOS[name](project)
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 239, in scenario_smoke_refs_do_not_expand
    subtree = p.orient(under=root)
              ^^^^^^^^^^^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 146, in orient
    return self.run_json(args)
           ^^^^^^^^^^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 98, in run_json
    text = proc.stdout.strip()
           ^^^^^^^^^^^^^^^^^
AttributeError: 'NoneType' object has no attribute 'strip'

:b shell 56
python tests/recursive/driver.py full/depth-chain-orient
:i returncode 1
:b stdout 0

:b stderr 1406
Exception in thread Thread-25 (_readerthread):
Traceback (most recent call last):
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1075, in _bootstrap_inner
    self.run()
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1012, in run
    self._target(*self._args, **self._kwargs)
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\subprocess.py", line 1601, in _readerthread
    buffer.append(fh.read())
                  ^^^^^^^^^
UnicodeDecodeError: 'gbk' codec can't decode byte 0xb3 in position 2368: illegal multibyte sequence
Traceback (most recent call last):
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 378, in <module>
    main(sys.argv[1:])
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 372, in main
    SCENARIOS[name](project)
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 267, in scenario_full_depth_chain
    mid_orient = p.orient(under=mid)
                 ^^^^^^^^^^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 146, in orient
    return self.run_json(args)
           ^^^^^^^^^^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 98, in run_json
    text = proc.stdout.strip()
           ^^^^^^^^^^^^^^^^^
AttributeError: 'NoneType' object has no attribute 'strip'

:b shell 57
python tests/recursive/driver.py full/reparent-join-leave
:i returncode 1
:b stdout 0

:b stderr 1433
Exception in thread Thread-9 (_readerthread):
Traceback (most recent call last):
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1075, in _bootstrap_inner
    self.run()
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1012, in run
    self._target(*self._args, **self._kwargs)
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\subprocess.py", line 1601, in _readerthread
    buffer.append(fh.read())
                  ^^^^^^^^^
UnicodeDecodeError: 'gbk' codec can't decode byte 0xb3 in position 951: illegal multibyte sequence
Traceback (most recent call last):
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 378, in <module>
    main(sys.argv[1:])
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 372, in main
    SCENARIOS[name](project)
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 295, in scenario_full_reparent_join_leave
    before = active_slugs(p.orient(under=root))
                          ^^^^^^^^^^^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 146, in orient
    return self.run_json(args)
           ^^^^^^^^^^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 98, in run_json
    text = proc.stdout.strip()
           ^^^^^^^^^^^^^^^^^
AttributeError: 'NoneType' object has no attribute 'strip'

:b shell 69
python tests/recursive/driver.py crash/complete-batch-strict-readable
:i returncode 1
:b stdout 0

:b stderr 1412
Exception in thread Thread-13 (_readerthread):
Traceback (most recent call last):
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1075, in _bootstrap_inner
    self.run()
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\threading.py", line 1012, in run
    self._target(*self._args, **self._kwargs)
  File "C:\Users\Admin\AppData\Local\Programs\Python\Python312\Lib\subprocess.py", line 1601, in _readerthread
    buffer.append(fh.read())
                  ^^^^^^^^^
UnicodeDecodeError: 'gbk' codec can't decode byte 0xb3 in position 898: illegal multibyte sequence
Traceback (most recent call last):
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 378, in <module>
    main(sys.argv[1:])
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 372, in main
    SCENARIOS[name](project)
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 330, in scenario_crash_complete_batch_readable
    orient = p.orient(under=root)
             ^^^^^^^^^^^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 146, in orient
    return self.run_json(args)
           ^^^^^^^^^^^^^^^^^^^
  File "D:\forks\tasktree-core\tests\recursive\driver.py", line 98, in run_json
    text = proc.stdout.strip()
           ^^^^^^^^^^^^^^^^^
AttributeError: 'NoneType' object has no attribute 'strip'

