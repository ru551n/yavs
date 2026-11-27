-- A clock copied from a field of a package signal (through an alias) and then by an entity
-- (`o <= i`): its edges come two deltas after those of its source, which a process makes from
-- waits in a procedure, so after a clock process's at coincident edges. Logic its registers feed
-- and the other clock samples joins; logic the other clock's registers feed and it samples stays
-- out (two levels: a register sampling one level a delta after the edge sees it settled). Against
-- the normal mode.
library ieee;
use ieee.std_logic_1164.all;
package cd_pkg is
  type cd_bus_t is record
    tck : std_ulogic;
    sel : std_ulogic;
  end record;
  signal cd_bus : cd_bus_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
entity cd_buf is
  port (i : in std_ulogic; o : out std_ulogic);
end entity;
architecture a of cd_buf is
begin
  o <= i;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.cd_pkg.all;
entity cd_tap is
  port (tck : out std_ulogic);
end entity;
architecture a of cd_tap is
  alias j : cd_bus_t is cd_bus;
begin
  tck <= j.tck;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cd_pkg.all;
entity cycle_copydelta is
end entity;
architecture t of cycle_copydelta is
  signal cc : std_ulogic := '0';
  signal tck, jc : std_ulogic;
  signal rc, rj, rk : unsigned(W - 1 downto 0) := (others => '0');
  signal xc1, xc, xj : unsigned(W - 1 downto 0);
begin
  pc : process
  begin
    wait for 5 ns;
    if now > 200 ns then
      wait;
    end if;
    cc <= not cc;
  end process;
  pj : process
    alias j : cd_bus_t is cd_bus;
    constant period : time := 5 ns;
    procedure tick(n : integer) is
    begin
      for i in 1 to n loop
        j.tck <= '0';
        wait for period / 2;
        j.tck <= '1';
        wait for period / 2;
      end loop;
    end procedure;
  begin
    -- Rising edges at multiples of 5 ns, with those of `cc`.
    wait for period / 2;
    j.sel <= '1';
    tick(4);
    j.sel <= '0';
    tick(36);
    wait;
  end process;
  tap : entity work.cd_tap port map (tck => tck);
  buf : entity work.cd_buf port map (i => tck, o => jc);
  gc : process (cc) begin if rising_edge(cc) then rc <= rc + 1 + xj; end if; end process;
  gj : process (jc) begin if rising_edge(jc) then rj <= rj + 3; end if; end process;
  gk : process (jc) begin if rising_edge(jc) then rk <= rk + xc; end if; end process;
  xj <= rj + 7;
  xc1 <= rc + 1;
  xc <= xc1 + 2;
  main : process
  begin
    for k in 1 to 20 loop
      wait for 10 ns;
      report "k=" & integer'image(k) & " " & to_hstring(rc) & " " & to_hstring(rj) & " " & to_hstring(rk);
    end loop;
    wait;
  end process;
end architecture;

-- Two copies in a row in the region would settle in one wake: one stays out (E4131), so each copy
-- still adds a delta.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cd_pkg.all;
entity cycle_copydelta2 is
end entity;
architecture t of cycle_copydelta2 is
  signal cc : std_ulogic := '0';
  signal tck, t2, jc : std_ulogic;
  signal rc, rj, rk : unsigned(W - 1 downto 0) := (others => '0');
  signal xc1, xc, xj : unsigned(W - 1 downto 0);
begin
  pc : process
  begin
    wait for 5 ns;
    if now > 200 ns then
      wait;
    end if;
    cc <= not cc;
  end process;
  pj : process
    alias j : cd_bus_t is cd_bus;
    constant period : time := 5 ns;
    procedure tick(n : integer) is
    begin
      for i in 1 to n loop
        j.tck <= '0';
        wait for period / 2;
        j.tck <= '1';
        wait for period / 2;
      end loop;
    end procedure;
  begin
    -- Rising edges at multiples of 5 ns, with those of `cc`.
    wait for period / 2;
    j.sel <= '1';
    tick(4);
    j.sel <= '0';
    tick(36);
    wait;
  end process;
  tap : entity work.cd_tap port map (tck => tck);
  t2 <= tck;
  buf : entity work.cd_buf port map (i => t2, o => jc);
  gc : process (cc) begin if rising_edge(cc) then rc <= rc + 1 + xj; end if; end process;
  gj : process (jc) begin if rising_edge(jc) then rj <= rj + 3; end if; end process;
  gk : process (jc) begin if rising_edge(jc) then rk <= rk + xc; end if; end process;
  xj <= rj + 7;
  xc1 <= rc + 1;
  xc <= xc1 + 2;
  main : process
  begin
    for k in 1 to 20 loop
      wait for 10 ns;
      report "k=" & integer'image(k) & " " & to_hstring(rc) & " " & to_hstring(rj) & " " & to_hstring(rk);
    end loop;
    wait;
  end process;
end architecture;

-- Waits for a formal of the procedure: elaboration does not know the time, so the clock keeps its
-- own delta class and the logic between the clocks stays out.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cd_pkg.all;
entity cycle_copydelta3 is
end entity;
architecture t of cycle_copydelta3 is
  signal cc : std_ulogic := '0';
  signal tck, jc : std_ulogic;
  signal rc, rj, rk : unsigned(W - 1 downto 0) := (others => '0');
  signal xc1, xc, xj : unsigned(W - 1 downto 0);
begin
  pc : process
  begin
    wait for 5 ns;
    if now > 200 ns then
      wait;
    end if;
    cc <= not cc;
  end process;
  pj : process
    alias j : cd_bus_t is cd_bus;
    constant period : time := 5 ns;
    procedure tick(n : integer; h : time := period / 2) is
    begin
      for i in 1 to n loop
        j.tck <= '0';
        wait for h;
        j.tck <= '1';
        wait for h;
      end loop;
    end procedure;
  begin
    -- Rising edges at multiples of 5 ns, with those of `cc`.
    wait for period / 2;
    j.sel <= '1';
    tick(4);
    j.sel <= '0';
    tick(36);
    wait;
  end process;
  tap : entity work.cd_tap port map (tck => tck);
  buf : entity work.cd_buf port map (i => tck, o => jc);
  gc : process (cc) begin if rising_edge(cc) then rc <= rc + 1 + xj; end if; end process;
  gj : process (jc) begin if rising_edge(jc) then rj <= rj + 3; end if; end process;
  gk : process (jc) begin if rising_edge(jc) then rk <= rk + xc; end if; end process;
  xj <= rj + 7;
  xc1 <= rc + 1;
  xc <= xc1 + 2;
  main : process
  begin
    for k in 1 to 20 loop
      wait for 10 ns;
      report "k=" & integer'image(k) & " " & to_hstring(rc) & " " & to_hstring(rj) & " " & to_hstring(rk);
    end loop;
    wait;
  end process;
end architecture;
