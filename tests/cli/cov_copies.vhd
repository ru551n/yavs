-- Coverage under per-instance copies, memoisation and inlining (crates/yavs/tests/coverage.rs
-- copies_count_alike): one counter row per instance, generate branches that fold away in some
-- copies, a package function inlined into the processes.
library ieee;
use ieee.std_logic_1164.all;

package cov_cp_pkg is
  function pick(x : natural; k : natural) return natural;
end package;

package body cov_cp_pkg is
  function pick(x : natural; k : natural) return natural is
  begin
    if x mod 2 = 0 and k > 1 then
      return x + k;
    elsif x = 3 then
      return 1;
    end if;
    return x;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.cov_cp_pkg.all;

-- Instances with different generics get copies of their own: the arms of MODE fold away.
entity cov_cp_sub is
  generic (MODE : natural);
  port (clk : in std_logic; y : out natural);
end entity;

architecture rtl of cov_cp_sub is
begin
  p : process (clk)
    variable v : natural := 0;
  begin
    if rising_edge(clk) then
      if MODE = 0 then
        v := pick(v + 1, 2);
      elsif MODE = 1 then
        v := v + 3;
      else
        v := pick(v, MODE);
      end if;
      y <= v;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.cov_cp_pkg.all;

entity cov_copies is
  generic (N : natural := 6; CYC : natural := 40);
end entity;

architecture sim of cov_copies is
  signal clk : std_logic := '0';
  signal q : std_logic_vector(N - 1 downto 0) := (others => '0');
  signal hit : std_logic_vector(N - 1 downto 0) := (others => '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  m : for k in 0 to 3 generate
    u : entity work.cov_cp_sub generic map (MODE => k) port map (clk => clk, y => open);
  end generate;
  g : for i in 0 to N - 1 generate
    p : process (clk)
      variable acc : natural := 0;
    begin
      if rising_edge(clk) then
        if i mod 3 = 0 then
          acc := pick(acc + i, i);
        else
          acc := pick(acc + 1, N);
        end if;
        case i mod 3 is
          when 0 => q(i) <= q((i + 1) mod N) xor '1';
          when 1 => q(i) <= q((i + 2) mod N) and not q(i);
          when others => q(i) <= q((i + 5) mod N) or q(i);
        end case;
        if acc > 20 and q(i) = '1' then
          hit(i) <= not hit(i);
        end if;
      end if;
    end process;
  end generate;
  main : process
  begin
    for c in 1 to CYC loop
      wait until rising_edge(clk);
    end loop;
    done <= true;
    wait;
  end process;
end architecture;
