-- Cycle mode: the region's outputs seen from outside, every clock and on events: a wait on an
-- output, on one element of a vector output, on a record field, on an output of more than 64
-- elements; an output assigned and assigned back in one run (no event); an output read by a
-- member and by the testbench; outputs a local function reads by name and local procedures wait
-- on, by name and through a formal; outputs a local function reads only in a declaration's
-- initial value and in an allocator's initial value; an output compared with the predefined `=`
-- of a local type; a wide output whose high word alone changes; a driver of the middle of a
-- vector (elements 20 to 79) that changes only above element 63; a record field after a field of
-- 60 elements.
-- `inner` is read by members only: it is not exported.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_export is
end entity;
architecture a of cycle_export is
  type rec_t is record
    a : std_ulogic;
    b : std_ulogic_vector(3 downto 0);
  end record;
  type state_t is (idle, busy);
  type pk_t is record
    a : std_ulogic_vector(59 downto 0);
    b : std_ulogic_vector(9 downto 0);
  end record;
  signal clk : std_ulogic := '0';
  signal st : state_t := idle;
  signal hw : std_ulogic_vector(99 downto 0);
  signal mid : std_ulogic_vector(99 downto 0) := (others => '0');
  signal pk : pk_t := ((others => '0'), (others => '0'));
  signal cnt : unsigned(7 downto 0) := (others => '0');
  signal v : std_ulogic_vector(7 downto 0);
  signal r : rec_t;
  signal w : std_ulogic_vector(99 downto 0) := (others => '0');
  signal pulse : std_ulogic := '0';
  signal both, inner : unsigned(7 downto 0);
  signal acc : unsigned(7 downto 0) := (others => '0');
  signal viaf, viap, viaformal, viad, vian : std_ulogic := '0';
  signal n_v3, n_ra, n_w, n_pulse, n_p, n_f, n_cnt, n_hw, n_pk, n_mid : natural := 0;

  impure function peek return std_ulogic is
  begin
    return viaf;
  end function;

  impure function peek_decl return std_ulogic is
    variable v : std_ulogic := viad;
  begin
    return v;
  end function;

  impure function peek_new return std_ulogic is
  begin
    return vian;
  end function;

  procedure wait_viap is
  begin
    wait on viap;
  end procedure;

  procedure wait_high(signal s : std_ulogic) is
  begin
    wait until s = '1';
  end procedure;
begin
  -- The region.
  reg : process (clk)
  begin
    if rising_edge(clk) then
      cnt <= cnt + 1;
      w <= w(98 downto 0) & not w(99);
      pulse <= '1';
      pulse <= '0';
      acc <= acc + inner + both;
      viaf <= cnt(1);
      viap <= cnt(2);
      viaformal <= cnt(3);
      viad <= cnt(4);
      vian <= cnt(5);
      pk.b <= std_ulogic_vector(cnt(7 downto 4)) & "000000";
    end if;
  end process;

  comb : process (cnt)
  begin
    v <= std_ulogic_vector(cnt) xor std_ulogic_vector(cnt(0) & cnt(7 downto 1));
    r <= (a => cnt(0) xor cnt(2), b => std_ulogic_vector(cnt(5 downto 2)));
    both <= cnt + 3;
    inner <= cnt + 5;
    st <= busy when cnt(0) = '1' else idle;
    hw <= std_ulogic_vector(cnt(3 downto 0)) & (95 downto 0 => '0');
    mid(79 downto 20) <= std_ulogic_vector(cnt(3 downto 0)) & (75 downto 20 => '0');
  end process;

  -- Outside the region (incomplete lists): the events of what they wait on.
  on_v3 : process (v(3))
  begin
    n_v3 <= n_v3 + 1;
  end process;

  on_ra : process (r.a)
  begin
    n_ra <= n_ra + 1;
  end process;

  on_w : process (w)
  begin
    n_w <= n_w + 1;
  end process;

  on_pulse : process (pulse)
  begin
    n_pulse <= n_pulse + 1;
  end process;

  on_hw : process (hw)
  begin
    n_hw <= n_hw + 1;
  end process;

  on_mid : process (mid)
  begin
    n_mid <= n_mid + 1;
  end process;

  on_pk : process (pk.b)
  begin
    n_pk <= n_pk + 1;
  end process;

  -- Outside (several waits): waits through local procedures.
  watcher : process
  begin
    wait_viap;
    n_p <= n_p + 1;
    wait_high(viaformal);
    n_f <= n_f + 1;
    wait on cnt;
    n_cnt <= n_cnt + 1;
  end process;

  main : process
    type logic_ptr is access std_ulogic;
    variable p : logic_ptr;
    variable sum, ones, nd, nn : natural := 0;
  begin
    for i in 1 to 40 loop
      clk <= '1';
      wait for 5 ns;
      sum := sum + to_integer(cnt) + to_integer(unsigned(v)) + to_integer(both)
             + to_integer(unsigned(r.b)) + to_integer(unsigned(hw(99 downto 96)))
             + to_integer(unsigned(pk.b(9 downto 6))) + to_integer(unsigned(mid(79 downto 76)));
      if peek = '1' then
        ones := ones + 1;
      end if;
      if peek_decl = '1' then
        nd := nd + 1;
      end if;
      p := new std_ulogic'(peek_new);
      if p.all = '1' then
        nn := nn + 1;
      end if;
      deallocate(p);
      if st = busy then
        ones := ones + 1000;
      end if;
      if w(99) = '1' then
        ones := ones + 100;
      end if;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "cnt=" & to_hstring(cnt) & " v=" & to_hstring(v) & " ra=" & to_string(r.a)
      & " rb=" & to_hstring(r.b) & " w=" & to_hstring(w) & " both=" & to_hstring(both)
      & " acc=" & to_hstring(acc) & " sum=" & to_string(sum) & " ones=" & to_string(ones)
      & " nd=" & to_string(nd) & " nn=" & to_string(nn);
    report "events v3=" & to_string(n_v3) & " ra=" & to_string(n_ra) & " w=" & to_string(n_w)
      & " pulse=" & to_string(n_pulse) & " p=" & to_string(n_p) & " f=" & to_string(n_f)
      & " cnt=" & to_string(n_cnt) & " hw=" & to_string(n_hw) & " pk=" & to_string(n_pk) & " mid=" & to_string(n_mid);
    wait;
  end process;
end architecture;
