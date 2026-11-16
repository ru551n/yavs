-- Combinational processes whose slices are bounded by variables of known values, against the
-- normal mode (`W` replaced by a width): `pack` packs fields by `lo`/`hi` (as a width converter
-- does), `walk` writes element `lo` of a vector in a loop of known iterations, `drop` picks its
-- branch by a function of a string generic that elaboration evaluates (a `while` loop), and only
-- the branch it picks assigns `dr_c` (no latch once the call folds). They
-- join; `two` gives `lo` another value on each path, so an element of `v2` keeps a value of the
-- previous run and the process stays out, and so does `elsy`, whose `elsif` reads `lo` as the
-- statement found it, not as its first branch leaves it, and `wl`, whose `lo` a `while` loop
-- changes.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_knownvar is
  generic (mode_g : string := "drop_only");
end entity;
architecture t of cycle_knownvar is
  constant with_strobe : boolean := true;
  constant with_last : boolean := true;
  type atom_t is record
    data : std_ulogic_vector(W - 1 downto 0);
    strobe : std_ulogic_vector(0 downto 0);
    last : std_ulogic;
  end record;
  type atom_vec_t is array (integer range <>) of atom_t;
  subtype packed_t is std_ulogic_vector(W + 1 downto 0);
  function same(a, b : string) return boolean is
    variable i : natural := 0;
  begin
    if a'length /= b'length then
      return false;
    end if;
    while i < a'length loop
      if a(a'low + i) /= b(b'low + i) then
        return false;
      end if;
      i := i + 1;
    end loop;
    return true;
  end function;
  signal clk, sel : std_ulogic := '0';
  signal d : unsigned(W - 1 downto 0) := (others => '0');
  signal atoms : atom_vec_t(0 to 1) := (others => ((others => '0'), "0", '0'));
  signal packed : std_ulogic_vector(2 * (W + 2) - 1 downto 0);
  signal w4, q4, two_y, el_y, wl_y : std_ulogic_vector(3 downto 0) := (others => '0');
  signal dr_a, dr_b, dr_c, rpk : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal rw4, rdr_a, rdr_b, rdr_c, rtwo, rel : std_ulogic_vector(W + 3 downto 0) := (others => '0');
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      atoms(0) <= (std_ulogic_vector(d), (0 => d(0)), d(W - 1));
      atoms(1) <= (std_ulogic_vector(not d), (0 => sel), d(0));
      q4 <= std_ulogic_vector(resize(d, 4));
      rpk <= packed(W - 1 downto 0) xor packed(2 * W + 1 downto W + 2);
      rw4(3 downto 0) <= w4;
      rdr_a(W - 1 downto 0) <= dr_a;
      rdr_b(W - 1 downto 0) <= dr_b;
      rdr_c(W - 1 downto 0) <= dr_c;
      rtwo(3 downto 0) <= two_y;
      rel(3 downto 0) <= el_y;
    end if;
  end process;
  pack : process (all)
    variable pk : packed_t := (others => '0');
    variable hi, lo : natural range packed_t'range := 0;
  begin
    for i in atoms'range loop
      lo := 0;
      hi := atoms(0).data'length - 1;
      pk(hi downto lo) := atoms(i).data;
      if with_strobe then
        lo := hi + 1;
        hi := lo + atoms(0).strobe'length - 1;
        pk(hi downto lo) := atoms(i).strobe;
      end if;
      if with_last then
        hi := hi + 1;
        pk(hi) := atoms(i).last;
      end if;
      packed((i + 1) * pk'length - 1 downto i * pk'length) <= pk;
    end loop;
  end process;
  walk : process (all)
    variable v : std_ulogic_vector(3 downto 0);
    variable lo : natural range 0 to 4 := 0;
  begin
    lo := 0;
    for i in 0 to 3 loop
      v(lo) := q4(3 - i);
      lo := lo + 1;
    end loop;
    w4 <= v;
  end process;
  drop : process (all)
  begin
    if not same(mode_g, "drop_only") then
      dr_a <= std_ulogic_vector(d);
      dr_b <= rpk;
    else
      dr_a <= rpk;
      dr_b <= not std_ulogic_vector(d);
      dr_c <= rpk xor std_ulogic_vector(d);
    end if;
  end process;
  two : process (all)
    variable v2 : std_ulogic_vector(3 downto 0) := (others => '0');
    variable lo : natural range 0 to 3 := 0;
  begin
    if sel = '1' then
      lo := 0;
    else
      lo := 1;
    end if;
    v2(lo) := q4(0);
    two_y <= v2;
  end process;
  elsy : process (all)
    variable v3 : std_ulogic_vector(3 downto 0) := (others => '0');
    variable lo : natural range 0 to 3 := 0;
  begin
    lo := 0;
    if sel = '1' then
      lo := 2;
      v3 := q4;
    elsif lo = 2 then
      v3 := not q4;
    end if;
    el_y <= v3;
  end process;
  wl : process (all)
    variable v4 : std_ulogic_vector(3 downto 0) := (others => '0');
    variable lo : natural range 0 to 3 := 0;
  begin
    lo := 0;
    while lo < 3 loop
      lo := lo + 1;
    end loop;
    v4(lo) := q4(0);
    wl_y <= v4;
  end process;
  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 12 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      d <= d(W - 2 downto 0) & r(0);
      sel <= r(3);
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " pk=" & to_hstring(rpk) & " w=" & to_hstring(rw4) &
        " a=" & to_hstring(rdr_a) & " b=" & to_hstring(rdr_b) & " c=" & to_hstring(rdr_c) &
        " two=" & to_hstring(rtwo) & " el=" & to_hstring(rel);
    end loop;
    wait;
  end process;
end architecture;
