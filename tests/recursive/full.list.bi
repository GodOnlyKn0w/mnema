:i count 2
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

