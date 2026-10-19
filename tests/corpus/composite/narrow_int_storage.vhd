-- difftest: top=narrow_int_storage std=2008
-- Integers of a 32-bit type are 4 bytes as array elements and in signals, 8 as scalar
-- variables and record fields (#415): negative values and the range ends through every path
-- that reads or writes them: element out/inout actuals, natives, implicit operations,
-- conversions to an array of a wider integer type, record and array signals, ports bound to
-- an element, 'last_value, binary files and aggregates. Individually associated signal
-- formals mixing scalar and element actuals are in process/signal_param_slices.
use std.textio.all;
package narrow_int_pkg is
  type big is range -2**40 to 2**40;
  type big_vec is array (natural range <>) of big;
  type rec is record
    a : integer;
    v : integer_vector(0 to 1);
  end record;
  type rec_vec is array (natural range <>) of rec;
  type mat is array (natural range <>, natural range <>) of integer;
  type ivfile is file of integer_vector;
  function img(v : integer_vector) return string;
end package;

package body narrow_int_pkg is
  function img(v : integer_vector) return string is
  begin
    if v'length = 0 then
      return "";
    elsif v'length = 1 then
      return integer'image(v(v'left));
    end if;
    return integer'image(v(v'left)) & "," & img(v(v'left + 1 to v'right));
  end function;
end package body;

use work.narrow_int_pkg.all;
entity narrow_int_cell is
  port (i : in integer; o : out integer);
end entity;
architecture a of narrow_int_cell is
begin
  o <= -i after 1 ns;
end architecture;

use std.textio.all;
use work.narrow_int_pkg.all;
entity narrow_int_storage is end;
architecture t of narrow_int_storage is
  signal sv : integer_vector(0 to 3) := (integer'low, -1, 0, integer'high);
  signal sr : rec := (a => -7, v => (-8, 9));
  signal srv : rec_vec(0 to 1) := (others => (a => -3, v => (-4, -5)));
  signal ins : integer_vector(0 to 1) := (-100, 200);
  signal outs : integer_vector(0 to 1);
  signal s0 : integer := -11;
  constant ck : integer_vector(0 to 2) := (-5, 7, integer'low);

  procedure dec(x : inout integer) is
  begin
    x := x - 1;
  end procedure;

  procedure set(variable x : out integer; v : integer) is
  begin
    x := v;
  end procedure;

begin
  g : for k in 0 to 1 generate
    u : entity work.narrow_int_cell port map (i => ins(k), o => outs(k));
  end generate;

  process
    variable v : integer_vector(1 to 4) := (integer'low, -2, 3, integer'high);
    variable m : mat(0 to 1, 0 to 2) := ((-1, -2, -3), (4, 5, -6));
    variable b : big_vec(1 to 4);
    variable l : line;
    variable rv : rec_vec(0 to 2) := (others => (a => -1, v => (-2, -3)));
    file f : ivfile;
    variable got : integer_vector(0 to 2);
    variable len : natural;
  begin
    file_open(f, "narrow_int_storage.bin", write_mode);
    write(f, (integer'low, -1, integer'high));
    file_close(f);
    file_open(f, "narrow_int_storage.bin", read_mode);
    read(f, got, len);
    file_close(f);
    report "file " & img(got) & " " & integer'image(len);
    assert got = (integer'low, -1, integer'high) severity failure;
    dec(v(3));
    set(v(2), -5);
    write(l, string'("-42 17"));
    read(l, v(3));
    report "v " & img(v) & " min " & integer'image(minimum(v))
      & " max " & integer'image(maximum(v));
    assert v = (integer'low, -5, -42, integer'high) severity failure;
    b := big_vec(v);
    b(1) := b(1) - 1;
    report "b " & big'image(b(1)) & " " & big'image(b(4));
    report "m " & integer'image(m(0, 2)) & " " & integer'image(m(1, 2))
      & " eq " & boolean'image(m = ((-1, -2, -3), (4, 5, -6)));
    v := v(3 to 4) & (-9, -10);
    report "cat " & img(v) & " lt " & boolean'image(v < (-42, 0, 0, 0));
    rv(1).v(1) := -99;
    report "rv " & integer'image(rv(1).v(1)) & " " & integer'image(rv(2).v(0))
      & " " & integer'image(rv(0).a);
    report "sv " & img(sv) & " sr " & integer'image(sr.a) & " "
      & integer'image(sr.v(0)) & " srv " & integer'image(srv(1).v(1));
    sv(1) <= -1000;
    sr <= (a => -70, v => (-80, 90));
    srv(0).v(0) <= -40;
    s0 <= -12;
    wait for 0 ns;
    report "sv " & img(sv) & " last " & integer'image(sv(1)'last_value)
      & " sr " & integer'image(sr.a) & " " & integer'image(sr.v(0))
      & " last " & integer'image(sr.v(0)'last_value)
      & " srv " & integer'image(srv(0).v(0)) & " s0 last " & integer'image(s0'last_value);
    wait for 2 ns;
    report "outs " & img(outs);
    ins <= (integer'high, integer'low + 1);
    wait for 2 ns;
    report "outs " & img(outs);
    assert outs = (-integer'high, integer'high) severity failure;
    report "ck " & integer'image(ck(0)) & " " & integer'image(ck(2));
    wait;
  end process;
end architecture;
