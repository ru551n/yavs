-- Variables read and deposited before their process runs (#342): a process variable, a
-- shared variable no process uses and one a process uses. The process copies its variable
-- to a signal when it first runs.
entity vars is end entity;

architecture sim of vars is
  signal seen : integer := -1;
  shared variable unused : integer := 7;
  shared variable used   : integer := 20;
begin
  p : process
    variable n : integer := 3;
  begin
    seen <= n;
    used := used + 1;
    wait for 1 ns;
    n := n + 1;
    seen <= n;
    wait;
  end process;
end architecture;
