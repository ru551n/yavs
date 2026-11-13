-- difftest: top=zero_compare std=2008
-- Element scans of a memory against fills: `x /= (x'range => '0')`, `(others => c)`,
-- `std_logic_vector(to_unsigned(0, W))`, named constants, over signals, external names and
-- variable snapshots; metavalues compare unequal under predefined equality (no warnings), descending
-- and null ranges, std_ulogic_vector and bit_vector, record fields, and the ordering relations.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package zero_compare_pkg is
  type slv_array_t is array (natural range <>) of std_logic_vector;
  type rec_t is record
    tag  : natural;
    data : std_logic_vector(7 downto 0);
    bits : bit_vector(0 to 3);
  end record;
  type rec_array_t is array (natural range <>) of rec_t;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.zero_compare_pkg.all;

entity zero_compare_ram is
  generic (R : positive; W : positive);
  port (clk : in std_logic; we : in std_logic; addr : in natural range 0 to R - 1;
        d : in std_logic_vector(W - 1 downto 0));
end entity;

architecture rtl of zero_compare_ram is
  signal mem : slv_array_t(0 to R - 1)(W - 1 downto 0) := (others => (others => '0'));
begin
  process (clk)
  begin
    if rising_edge(clk) and we = '1' then
      mem(addr) <= d;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.zero_compare_pkg.all;

entity zero_compare is
  generic (R : positive := 6; W : positive := 12);
end entity;

architecture tb of zero_compare is
  subtype word_t is std_logic_vector(W - 1 downto 0);
  subtype slv is std_logic_vector;
  constant ZW : word_t := (others => '0');
  signal clk, we : std_logic := '0';
  signal addr : natural range 0 to R - 1 := 0;
  signal d : word_t := (others => '0');
  signal own : slv_array_t(0 to R - 1)(W - 1 downto 0) := (others => (others => 'U'));
  subtype up_t is std_logic_vector(0 to 69);
  subtype bv_t is bit_vector(9 downto 2);
  signal up : slv_array_t(1 to 3)(0 to 69) := (others => (others => '0'));
  signal recs : rec_array_t(0 to 2) := (others => (0, x"00", "0000"));
  signal bv : bit_vector(9 downto 2) := (others => '0');
  signal uv : std_ulogic_vector(0 to 4) := "00000";

  -- Count the elements that are not all '0', in five spellings each, and check they agree.
  procedure scan(name : string; signal m : in slv_array_t; n : out natural) is
    variable a, b, c, e, f : natural := 0;
    variable snap : slv_array_t(m'range)(m(m'low)'range);
  begin
    snap := m;
    for i in m'range loop
      if m(i) /= (m(i)'range => '0') then a := a + 1; end if;
      if not (m(i) = (m(i)'reverse_range => '0')) then b := b + 1; end if;
      if m(i) /= std_logic_vector(to_unsigned(0, m(i)'length)) then c := c + 1; end if;
      if snap(i) /= (snap(i)'range => '0') then e := e + 1; end if;
      if (snap(i)'range => '0') /= snap(i) then f := f + 1; end if;
    end loop;
    report name & ": " & integer'image(a) & integer'image(b) & integer'image(c) &
           integer'image(e) & integer'image(f);
    assert a = b and b = c and c = e and e = f report name & ": spellings disagree" severity failure;
    n := a;
  end procedure;
begin
  u_ram : entity work.zero_compare_ram
    generic map (R => R, W => W)
    port map (clk => clk, we => we, addr => addr, d => d);

  p_main : process
    alias mem is <<signal .zero_compare.u_ram.mem : slv_array_t(0 to R - 1)(W - 1 downto 0)>>;
    variable n : natural;
    variable v : word_t;
    variable nul : std_logic_vector(3 downto 4);
    procedure tick is
    begin
      clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns;
    end procedure;
    procedure check_ext(want : natural) is
      variable k : natural := 0;
    begin
      for i in mem'range loop
        if mem(i) /= (mem(i)'range => '0') then k := k + 1; end if;
      end loop;
      report "external: " & integer'image(k);
      assert k = want severity failure;
    end procedure;
  begin
    wait for 1 ns;
    check_ext(0);
    scan("own all U", own, n);
    assert n = R severity failure;
    own <= (others => (others => '0'));
    wait for 0 ns;
    scan("own zero", own, n);
    assert n = 0 severity failure;
    -- One metavalue per word in a different place: '1', 'U', 'X', '-', 'Z', 'L'.
    own(0)(0) <= '1';
    own(1)(W - 1) <= 'U';
    own(2)(3) <= 'X';
    own(3)(5) <= '-';
    own(4)(W / 2) <= 'Z';
    own(5)(1) <= 'L';
    wait for 0 ns;
    scan("own metavalues", own, n);
    assert n = R severity failure;
    -- '-' is not equal to '0' under predefined equality, unlike ?=.
    assert (own(3) ?= ZW) = '1' severity failure;
    assert own(3) /= ZW severity failure;
    -- Writes through the RAM, seen through the external name.
    we <= '1'; addr <= 2; d <= (4 => 'H', others => '0'); tick;
    addr <= 5; d <= (others => '0'); tick;
    we <= '0'; tick;
    check_ext(1);
    -- A wide element (70 > 64 elements), ascending.
    for i in up'range loop
      assert up(i) = (up(i)'range => '0') severity failure;
    end loop;
    up(2)(69) <= '1';
    wait for 0 ns;
    assert up(1) = up_t'(others => '0') and up(2) /= (up(2)'range => '0') and up(3) = (0 to 69 => '0')
      severity failure;
    up(2) <= (others => '0');
    up(3)(0) <= 'W';
    wait for 0 ns;
    assert up(2) = (up(2)'range => '0') and up(3) /= (up(3)'range => '0') severity failure;
    -- Fills with other values and constants, compared both ways.
    v := (others => '1');
    assert v = (v'range => '1') and v /= (v'range => '0') and (v'range => 'H') /= v severity failure;
    v := (others => 'Z');
    assert v = word_t'(others => 'Z') and v /= word_t'(others => 'X') severity failure;
    -- Null arrays: equal to any null fill, unequal to a non-null one.
    assert nul = (nul'range => '1') and nul = (3 downto 4 => '0') severity failure;
    assert nul /= (0 to 0 => '0') severity failure;
    -- A fill of another length is unequal, not an error.
    assert d /= (0 to W => '0') and d /= (1 to W - 1 => '0') severity failure;
    -- bit_vector and std_ulogic_vector.
    assert bv = (bv'range => '0') and uv = (uv'range => '0') severity failure;
    bv(5) <= '1'; uv(4) <= 'U';
    wait for 0 ns;
    assert bv /= (bv'range => '0') and bv /= bv_t'(others => '0') and uv /= (uv'range => '0')
      severity failure;
    -- Record fields against fills.
    for i in recs'range loop
      assert recs(i).data = (recs(i).data'range => '0') and recs(i).bits = bit_vector'(0 to 3 => '0')
        severity failure;
    end loop;
    recs(1).data(7) <= 'X'; recs(2).bits(0) <= '1';
    wait for 0 ns;
    n := 0;
    for i in recs'range loop
      if recs(i).data /= (recs(i).data'range => '0') or recs(i).bits /= (recs(i).bits'range => '0') then
        n := n + 1;
      end if;
    end loop;
    report "records: " & integer'image(n);
    assert n = 2 severity failure;
    -- Ordering relations of logic arrays: lexicographic by element position, enumeration order.
    assert slv'("0U") < "00" and slv'("01") > "0X" and slv'("1") > "0111" and slv'("") < "0"
      and slv'("0") <= "0" and slv'("ZZ") >= "Z1" and not (slv'("UU") > "UU") severity failure;
    assert slv'("1-") > "1U" and slv'("X") < "0" and slv'("10") >= "1" severity failure;
    assert bit_vector'("10") > "011" and bit_vector'("") <= "" severity failure;
    report "zero_compare done";
    wait;
  end process;
end architecture;
