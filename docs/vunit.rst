VUnit
=====

`VUnit <https://vunit.github.io>`__ runs testbenches on yavs through a yavs simulator interface.
The interface is not in a VUnit release yet: until it is, a released VUnit cannot select yavs.
Once it is:

.. code-block:: bash

    VUNIT_SIMULATOR=yavs python run.py

with ``yavs`` on ``PATH`` or its directory in ``VUNIT_YAVS_PATH``. Each yavs runs with
``--threads 1`` under VUnit's ``-p N``; add ``--threads`` through the ``yavs.sim_flags``
simulation option to change that. Mixing VHDL-2008 and VHDL-2019 files in one VUnit project is
allowed.
