-- difftest: top=stack_nested_index exit=1 ignore=fatalmsg
-- ignore=fatalmsg: yavs words the failure of an index in its own way.
-- An index past a descending parameter in a procedure nested in a process, called from what may be
-- a copy, after it has run in range for a while: the message names the parameter's bounds.
library ieee;
use ieee.std_logic_1164.all;

entity stack_nested_inst is
  generic (K : natural; W : positive);
  port (clk : in std_logic; o : out natural);
end entity;

architecture a of stack_nested_inst is
begin
  process (clk)
    variable cnt : natural := 0;
    variable t : natural := 0;
    variable v : std_logic_vector(W + 1 downto 2) := (others => '1');
    procedure read_it(a : in std_logic_vector; i : in natural; variable x : inout natural) is
      procedure inner is
      begin
        if a(i) = '1' then x := x + a'length; end if;
      end procedure;
    begin
      inner;
    end procedure;
  begin
    if rising_edge(clk) then
      cnt := cnt + 1;
      if cnt < 200 + K then
        read_it(v, 2 + cnt mod W, t);
      else
        read_it(v, W + 2 + K, t);
      end if;
      o <= t;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity stack_nested_index is
end entity;

architecture a of stack_nested_index is
  signal clk : std_logic := '0';
  signal o0, o1 : natural := 0;
begin
  clk <= not clk after 5 ns when now < 3 us else '0';
  i0 : entity work.stack_nested_inst generic map (0, 5) port map (clk, o0);
  i1 : entity work.stack_nested_inst generic map (3, 9) port map (clk, o1);
end architecture;
