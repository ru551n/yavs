-- VHDL-2008: `process (all)` and `to_string`, in one design with the 1993 package
use work.old_pkg;

entity mixed_tb is
end entity;

architecture sim of mixed_tb is
  signal a : integer := old_pkg.start;
  signal b : integer;
begin
  process (all)
  begin
    b <= a + 1;
  end process;

  process
  begin
    wait for 1 ns;
    report "b is " & to_string(b);
    std.env.finish;
  end process;
end architecture;
