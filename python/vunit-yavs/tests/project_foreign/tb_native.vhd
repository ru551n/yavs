library vunit_lib;
context vunit_lib.vunit_context;
library native_demo;
use native_demo.native_pkg.all;

entity tb_native is
  generic (runner_cfg : string);
end entity;

architecture a of tb_native is
begin
  main : process
  begin
    test_runner_setup(runner, runner_cfg);
    while test_suite loop
      if run("A native library function is called") then
        check_equal(native_add1(41), 42);
      end if;
    end loop;
    test_runner_cleanup(runner);
  end process;
end architecture;
