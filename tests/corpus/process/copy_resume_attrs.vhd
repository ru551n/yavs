-- difftest: top=copy_resume_attrs
-- Monitors of generate lanes that wait on an element of an array signal and read its attributes
-- (#1138): an element of an array of records with an unconstrained field, a field of a nested record
-- and a slice, all indexed by the generate parameter. A process switches to its specialised copy
-- between two activations (after 1024 of them by default); the copy reads the parameter at run time,
-- and its first activation must see this lane's element. The first and the last element never
-- change (a logic leaf keeps its elements in reverse order), so reading either by mistake fails an
-- assertion. Each monitor runs about 1500 activations, past the switch.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package copy_resume_attrs_pkg is
  type axis_t is record
    tvalid : std_logic;
    tdata  : std_logic_vector;
  end record;
  type axis_array_t is array (natural range <>) of axis_t;
  type inner_t is record
    a : std_logic_vector(3 downto 0);
    b : integer;
  end record;
  type outer_t is record
    x : inner_t;
    f : std_logic;
  end record;
  type outer_array_t is array (natural range <>) of outer_t;
  type int_array_t is array (natural range <>) of integer;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.copy_resume_attrs_pkg.all;

entity copy_resume_attrs is
  generic (N : positive := 3; W : positive := 8);
end entity;

architecture t of copy_resume_attrs is
  signal clk  : std_logic := '0';
  signal done : boolean := false;
  signal cnt  : unsigned(W - 1 downto 0) := (others => '0');
  signal ch   : axis_array_t(0 to N + 1)(tdata(W - 1 downto 0)) := (others => ('0', (others => '0')));
  signal o    : outer_array_t(0 to N + 1) := (others => (("0000", 0), '0'));
  signal sv   : std_logic_vector(0 to 4 * N + 7) := (others => '0');
  signal n_ch : int_array_t(1 to N) := (others => 0);
  signal n_o  : int_array_t(1 to N) := (others => 0);
  signal n_sv : int_array_t(1 to N) := (others => 0);
  signal sum  : int_array_t(1 to N) := (others => 0);
begin
  clk <= not clk after 5 ns when not done;

  process (clk) begin
    if rising_edge(clk) then
      cnt <= cnt + 1;
    end if;
  end process;

  g : for i in 0 to N - 1 generate
    process (clk) begin
      if rising_edge(clk) then
        ch(i + 1).tdata <= std_logic_vector(cnt + i + 1);
        ch(i + 1).tvalid <= cnt(0);
        o(i + 1).x.a <= std_logic_vector(cnt(3 downto 0) + i);
        o(i + 1).x.b <= to_integer(cnt);
        sv(4 * i + 4 to 4 * i + 7) <= std_logic_vector(cnt(3 downto 0) + 2 * i + 2);
      end if;
    end process;

    -- The issue's monitor: the whole record element, its 'active and the 'event of a field.
    process
      variable k : natural := 0;
      variable acc : natural := 0;
    begin
      wait on ch(i + 1);
      assert ch(i + 1)'active report "lane " & integer'image(i + 1) & ": record element not active" severity failure;
      assert ch(i + 1).tdata'event report "lane " & integer'image(i + 1) & ": no event on tdata" severity failure;
      assert ch(i + 1).tdata'last_event = 0 ns
        report "lane " & integer'image(i + 1) & ": last_event " & time'image(ch(i + 1).tdata'last_event) severity failure;
      if k > 0 then
        acc := (acc + to_integer(unsigned(ch(i + 1).tdata'last_value)) + ch(i + 1).tvalid'last_event / 1 ns) mod 65536;
      end if;
      k := k + 1;
      n_ch(i + 1) <= k;
      sum(i + 1) <= acc;
    end process;

    -- A field of a nested record of an element.
    process
      variable k : natural := 0;
    begin
      wait on o(i + 1).x;
      assert o(i + 1).x.a'event and o(i + 1).x'active
        report "lane " & integer'image(i + 1) & ": nested field not changed" severity failure;
      assert o(i + 1).x.a'last_value /= o(i + 1).x.a
        report "lane " & integer'image(i + 1) & ": nested last_value" severity failure;
      k := k + 1;
      n_o(i + 1) <= k;
    end process;

    -- A slice of a vector.
    process
      variable k : natural := 0;
    begin
      wait on sv(4 * i + 4 to 4 * i + 7);
      assert sv(4 * i + 4 to 4 * i + 7)'event
        report "lane " & integer'image(i + 1) & ": slice not changed" severity failure;
      k := k + 1;
      n_sv(i + 1) <= k;
    end process;
  end generate;

  process begin
    wait for 15 us;
    done <= true;
    wait for 1 ns;
    for i in 1 to N loop
      report "lane " & integer'image(i) & ": ch " & integer'image(n_ch(i)) & " sum " & integer'image(sum(i))
        & " o " & integer'image(n_o(i)) & " sv " & integer'image(n_sv(i));
    end loop;
    wait;
  end process;
end architecture;
