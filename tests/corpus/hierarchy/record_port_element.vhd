-- difftest: top=record_port_element std=2008 ignore=order
-- Record ports on elements of an array of records, one lane per generate iteration through a
-- component: an `in` record (nested) on `ch(i)`, an `out` record on `ch(i + 1)` that is the
-- only source of that element, an `out` copy of a sub-record on an array of sub-records, a
-- `buffer` read inside, an `out` record with a null field (it collapses too, its null leaf
-- onto the actual's), and a lane whose `out` element has a second driver of a resolved field
-- (it keeps its edges). The 24-bit data field puts lanes across the 64-element words of the
-- leaf. Read through the formal and
-- the actual: values, 'event, 'active, 'last_value, 'driving and 'driving_value, the formal's
-- default against the actual's initial value (the actual starts at the formal's; 'last_value
-- before the first event is left out: the references differ there). Same-cycle reports of
-- different processes are unordered.
library ieee;
use ieee.std_logic_1164.all;

package record_port_element_pkg is
  type inner_t is record
    v : std_logic;
    d : std_logic_vector(23 downto 0);
  end record;
  type rec_t is record
    h : inner_t;
    n : integer;
    k : bit_vector(2 downto 0);
  end record;
  -- A record with a null field: that field has no port edge and no element.
  type null_t is record
    a : bit;
    z : std_logic_vector(0 downto 1);
  end record;
  type null_arr_t is array (natural range <>) of null_t;
  type rec_arr_t is array (natural range <>) of rec_t;
  type inner_arr_t is array (natural range <>) of inner_t;
  type int_arr_t is array (natural range <>) of integer;
  constant REC_INIT : rec_t := ((v => '1', d => x"123456"), 3, "011");
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.record_port_element_pkg.all;

entity record_port_element_stage is
  generic (IDX : natural);
  port (clk : in std_logic;
        s : in rec_t;
        m : out rec_t := ((v => '0', d => x"0000aa"), -1, "101");
        c : out inner_t;
        b : buffer integer := 100;
        y : out null_t := ('1', (others => '0')));
end;
architecture t of record_port_element_stage is
begin
  process (clk) begin
    if rising_edge(clk) then
      m.h.v <= s.h.v;
      m.h.d <= std_logic_vector(unsigned(s.h.d) + IDX + 1);
      m.n <= s.n + 1;
      m.k <= s.k rol 1;
      b <= b + s.n;
      y.a <= not y.a;
      report "lane " & integer'image(IDX) & " drv d=" & to_hstring(m.h.d'driving_value) &
        " n=" & integer'image(m.n'driving_value) & " driving=" & boolean'image(m.h.v'driving) &
        " b=" & integer'image(b);
    end if;
  end process;
  c <= s.h;
  mon : process (s, m)
    variable lv : integer;
  begin
    if now > 0 ns then
      lv := s.n;
      if s.n'event then
        lv := s.n'last_value;
      end if;
      report "lane " & integer'image(IDX) & " s.d=" & to_hstring(s.h.d) & " s.v=" &
        to_string(s.h.v) & " s'event=" & boolean'image(s'event) & " s.n'last_value=" &
        integer'image(lv) & " m.d=" & to_hstring(m.h.d) & " m.n=" & integer'image(m.n) &
        " m'active=" & boolean'image(m'active) & " m.k=" & to_string(m.k) & " m.h'event=" &
        boolean'image(m.h'event) & " y.a=" & bit'image(y.a) & " y.z'length=" &
        integer'image(y.z'length);
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.record_port_element_pkg.all;

entity record_port_element is end;
architecture t of record_port_element is
  component record_port_element_stage is
    generic (IDX : natural);
    port (clk : in std_logic;
          s : in rec_t;
          m : out rec_t := ((v => '0', d => x"0000aa"), -1, "101");
          c : out inner_t;
          b : buffer integer := 100;
          y : out null_t := ('1', (others => '0')));
  end component;
  signal clk : std_logic := '0';
  signal ch : rec_arr_t(0 to 5) := (others => REC_INIT);
  signal cc : inner_arr_t(0 to 4);
  signal bb : int_arr_t(0 to 5) := (others => 7);
  signal t : rec_arr_t(0 to 1) := (others => REC_INIT);
  signal ny : null_arr_t(0 to 5);
begin
  g : for i in 0 to 4 generate
    u : record_port_element_stage generic map (IDX => i)
      port map (clk => clk, s => ch(i), m => ch(i + 1), c => cc(i), b => bb(i), y => ny(i));
  end generate;
  -- `t(1).h.v` has a second source: the lane's `m` keeps its edges.
  x : entity work.record_port_element_stage generic map (IDX => 9)
    port map (clk => clk, s => ch(5), m => t(1), c => open, b => bb(5), y => ny(5));
  t(1).h.v <= 'L';

  mon : process (ch, t)
    variable lv : integer;
  begin
    if now > 0 ns then
      lv := ch(3).n;
      if ch(3).n'event then
        lv := ch(3).n'last_value;
      end if;
      report "top ch(2).d=" & to_hstring(ch(2).h.d) & " ch(2)'event=" &
        boolean'image(ch(2)'event) & " ch(3).n=" & integer'image(ch(3).n) &
        " ch(3).n'last_value=" & integer'image(lv) & " ch(5)'active=" &
        boolean'image(ch(5)'active) & " t(1).v=" & to_string(t(1).h.v) & " t(1).d=" &
        to_hstring(t(1).h.d);
    end if;
  end process;

  process
    variable d : unsigned(23 downto 0) := x"fffff0";
  begin
    wait for 1 ns;
    for k in 1 to 6 loop
      d := d + 7;
      ch(0).h.d <= std_logic_vector(d);
      ch(0).h.v <= not ch(0).h.v;
      ch(0).n <= k;
      ch(0).k <= ch(0).k rol 1;
      clk <= '1'; wait for 5 ns;
      clk <= '0'; wait for 5 ns;
    end loop;
    report "end ch(5).d=" & to_hstring(ch(5).h.d) & " ch(5).n=" & integer'image(ch(5).n) &
      " cc(4).d=" & to_hstring(cc(4).d) & " bb(3)=" & integer'image(bb(3)) & " t(1).n=" &
      integer'image(t(1).n) & " ny(2).a=" & bit'image(ny(2).a);
    wait;
  end process;
end;
