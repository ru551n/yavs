-- Direct drivers assigned again in the same activation: a default then an override of a
-- std_logic, loops of drives to one std_logic (8 and 40 of them: compiled code hands its drives
-- over 32 at a time), an integer register assigned twice, a byte vector assigned whole twice and
-- then one element of it. Each later zero-delay drive of the same element replaces the run's
-- entry for it instead of adding one (`direct_codes`, `direct_planes` and `direct_bytes` count
-- the entries committed).
library ieee;
use ieee.std_logic_1164.all;

entity redrive is end;
architecture a of redrive is
  signal clk : std_logic := '0';
  signal o, q, z : std_logic := '0';
  signal n : integer := 0;
  signal v : std_logic_vector(7 downto 0) := (others => '0');
begin
  stim : process
    variable sum : integer := 0;
  begin
    for i in 1 to 100 loop
      clk <= '1';
      wait for 1 ns;
      clk <= '0';
      wait for 1 ns;
      if o = '1' then sum := sum + i; end if;
      if q = '1' then sum := sum + 2; end if;
      if z = '1' then sum := sum + 4; end if;
      if v(3) = '1' then sum := sum + 3; end if;
      if v(0) = '1' then sum := sum + 5; end if;
      sum := sum + n mod 7;
    end loop;
    report "sum=" & integer'image(sum);
    wait;
  end process;

  regs : process (clk)
    variable k : integer := 0;
  begin
    if rising_edge(clk) then
      k := k + 1;
      o <= '0';
      if k mod 3 /= 0 then
        o <= '1';
      end if;
      for i in 0 to 7 loop
        if (i + k) mod 2 = 0 then q <= '1'; else q <= '0'; end if;
      end loop;
      for i in 0 to 39 loop
        if i mod 3 = k mod 2 then z <= '1'; else z <= '0'; end if;
      end loop;
      n <= k;
      n <= n + k;
      v <= (others => '0');
      if k mod 2 = 0 then
        v <= (others => '1');
        v(0) <= '0';
      end if;
    end if;
  end process;
end;
