library vunit_lib;
context vunit_lib.vunit_context;
library native_demo;
use native_demo.native_pkg.all;

entity tb_python is
  generic (runner_cfg : string);
end entity;

architecture a of tb_python is
begin
  main : process
  begin
    test_runner_setup(runner, runner_cfg);
    while test_suite loop
      if run("Python is called from VHDL") then
        check_equal(exec("def square(x): return x * x"), 0);
        check_equal(eval("square(12) + 1"), 145);
        check_equal(eval("len('abc')"), 3);
      elsif run("A Python error is printed by Python and returned") then
        check_equal(exec("raise ValueError('expected')"), 1);
        check_equal(eval("1 + 1"), 2);
      end if;
    end loop;
    test_runner_cleanup(runner);
  end process;
end architecture;
