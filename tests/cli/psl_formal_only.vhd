-- PSL for formal tools only: nothing to run in simulation, skipped without a warning.
library ieee;
use ieee.std_logic_1164.all;
entity psl_formal_only is end entity;
architecture rtl of psl_formal_only is
  signal clk : std_logic := '0';
  signal a, b : std_logic := '0';
begin
  default clock is rising_edge(clk);
  r1: restrict {a = '0'; a = '1'};
  r2: restrict_guarantee {a = '0'; a = '1'};
  g1: assume_guarantee always (a = '0');
  f1: fairness (a = '1');
  f2: strong fairness (a = '1'), (b = '0');
  n1: assume always (a = '0' or nondet(0, 1) = 0);
  ok: assert always (b = '0') report "b is high";
end architecture;
