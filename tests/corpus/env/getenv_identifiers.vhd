-- difftest: top=getenv_identifiers std=2019
-- getenv of a conditional analysis identifier gives its value, ahead of the environment (#917).
use std.env.all;
use std.textio.all;
entity getenv_identifiers is end entity;
architecture a of getenv_identifiers is
begin
  process
    variable value : line;
  begin
    value := getenv("TOOL_NAME");
    assert value.all = TOOL_NAME severity failure;
    assert getenv("VHDL_VERSION") = VHDL_VERSION severity failure;
    assert getenv("TOOL_TYPE") = TOOL_TYPE severity failure;
    assert getenv("TOOL_EDITION") = TOOL_EDITION severity failure;
    assert getenv("TOOL_VERSION") = TOOL_VERSION severity failure;
    assert getenv("YAVS_TEST_SURELY_UNSET") = "" severity failure;
    report "done";
    wait;
  end process;
end architecture;
