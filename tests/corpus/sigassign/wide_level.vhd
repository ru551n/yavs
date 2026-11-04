-- difftest: top=wide_level ignore=order
-- ignore=order: several processes report in one delta cycle, in an order the LRM leaves open.
-- Readers of a bus element that name no attribute, which the kernel still answers from the element's
-- event and old value: the level of a `wait until` on the element or on a scalar port collapsed into
-- it, on `std_logic` and on `bit`, an edge through a port, an implicit signal alone on a bus. A bus
-- that keeps less must keep these.
library ieee;
use ieee.std_logic_1164.all;

entity wide_level_port is
  port (clk : in std_logic; rst : in std_logic; d : in std_logic; q : out std_logic);
end entity;

architecture a of wide_level_port is
begin
  process
  begin
    wait until clk = '1';
    report "port level 1";
  end process;

  process
  begin
    wait until clk = '0';
    report "port level 0";
  end process;

  process (clk, rst)
  begin
    if rst = '1' then
      q <= '0';
    elsif rising_edge(clk) then
      q <= d;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity wide_level is
end entity;

architecture t of wide_level is
  signal s, u, m : std_logic_vector(99 downto 0) := (others => '0');
  signal st, qu, tr, de : std_logic_vector(99 downto 0) := (others => '0');
  signal b : bit_vector(99 downto 0);
  signal q : std_logic;
  alias e3 : std_logic is s(3);

begin
  process (st'stable)
  begin
    report "stable " & boolean'image(st'stable);
  end process;

  process (qu'quiet)
  begin
    report "quiet " & boolean'image(qu'quiet);
  end process;

  process (tr'transaction)
  begin
    report "transaction";
  end process;

  process (de'delayed(2 ns))
    variable v : std_logic_vector(99 downto 0);
  begin
    v := de'delayed(2 ns);
    -- Its value before 2 ns: the references differ, see the catalogue.
    if now > 3 ns then
      report "delayed " & std_logic'image(v(70));
    end if;
  end process;

  b <= to_bitvector(s);
  p : entity work.wide_level_port port map (clk => u(70), rst => u(3), d => u(4), q => q);

  process (q)
  begin
    report "q " & std_logic'image(q);
  end process;

  process
  begin
    wait until s(70) = '1';
    report "element level 1";
  end process;

  process
  begin
    wait until s(70) = '0';
    report "element level 0";
  end process;

  process
  begin
    wait on s(70) until s(70) = '1';
    report "on element level 1";
  end process;

  process
  begin
    wait until e3 = '1';
    report "alias level 1";
  end process;

  process
  begin
    wait until b(70) = '1';
    report "bit level 1";
  end process;

  process
  begin
    wait until m(70) = '1' for 100 ns;
    report "level or timeout";
    wait until m(3) = '0';
    report "second wait of the process";
    wait;
  end process;

  stim : process
    procedure step (v : std_logic_vector(99 downto 0)) is
    begin
      s <= v;
      u <= v;
      m <= v;
      st <= v;
      qu <= v;
      tr <= v;
      de <= v;
      wait for 10 ns;
    end procedure;
    variable v : std_logic_vector(99 downto 0) := (others => '0');
  begin
    wait for 5 ns;
    step((others => '1'));
    step((others => '1'));
    step((others => '0'));
    v(70) := '1';
    step(v);
    v(4) := '1';
    step(v);
    v(70) := '0';
    step(v);
    v(70) := '1';
    v(4) := '0';
    v(3) := '1';
    step(v);
    v := (others => 'Z');
    step(v);
    report "done";
    wait;
  end process;
end architecture;
