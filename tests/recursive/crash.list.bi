:i count 1
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

