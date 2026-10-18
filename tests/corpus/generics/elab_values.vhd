-- difftest: top=elab_values std=2008
-- Composite values evaluated during elaboration through the interpreter (#220): multi-
-- dimensional aggregates, arrays of composites (arrays of records, arrays of arrays), records
-- with array elements, shift operators and TO_STRING of real and physical values, as generic
-- map actuals, constant values and signal bounds. Impure initial values of these types run in
-- declaration order among the other elaboration-time calls (LRM 14.4.2): the ids show it.
package elab_values_pkg is
  type matrix is array (natural range <>, natural range <>) of integer;
  type pair is record
    name : string(1 to 2);
    n    : integer;
  end record;
  type pair_vec is array (natural range <>) of pair;
  type word_vec is array (natural range <>) of bit_vector(3 downto 0);
  type dist is range 0 to 1000000
    units
      mm;
      cm = 10 mm;
    end units;
  function total(m : matrix) return integer;
  function transpose(m : matrix) return matrix;
  function names(v : pair_vec) return string;
  function mk_pairs(n : natural) return pair_vec;
  function swap(p : pair) return pair;
  function xor_all(v : word_vec) return bit_vector;
  type counter_t is protected
    impure function bump return natural;
    impure function count return natural;
  end protected;
  impure function next_id return natural;
  impure function new_pair(s : string) return pair;
  impure function new_pairs(n : natural) return pair_vec;
  impure function ids return natural;
end package;

package body elab_values_pkg is
  function total(m : matrix) return integer is
    variable s : integer := 0;
  begin
    for i in m'range(1) loop
      for j in m'range(2) loop
        s := s + m(i, j);
      end loop;
    end loop;
    return s;
  end function;
  function transpose(m : matrix) return matrix is
    variable r : matrix(m'range(2), m'range(1));
  begin
    for i in m'range(1) loop
      for j in m'range(2) loop
        r(j, i) := m(i, j);
      end loop;
    end loop;
    return r;
  end function;
  function names(v : pair_vec) return string is
    variable r : string(1 to 2 * v'length);
  begin
    for i in 0 to v'length - 1 loop
      r(2 * i + 1 to 2 * i + 2) := v(v'left + i).name;
    end loop;
    return r;
  end function;
  function mk_pairs(n : natural) return pair_vec is
    variable r : pair_vec(1 to n);
  begin
    for i in r'range loop
      r(i) := (name => character'val(96 + i) & character'val(64 + i), n => i * 10);
    end loop;
    return r;
  end function;
  function swap(p : pair) return pair is
  begin
    return (name => p.name(2) & p.name(1), n => -p.n);
  end function;
  function xor_all(v : word_vec) return bit_vector is
    variable r : bit_vector(3 downto 0) := "0000";
  begin
    for i in v'range loop
      r := r xor v(i);
    end loop;
    return r;
  end function;

  type counter_t is protected body
    variable n : natural := 0;
    impure function bump return natural is
    begin
      n := n + 1;
      return n;
    end function;
    impure function count return natural is
    begin
      return n;
    end function;
  end protected body;

  shared variable c : counter_t;

  impure function next_id return natural is
  begin
    return c.bump;
  end function;
  impure function new_pair(s : string) return pair is
  begin
    return (name => s, n => c.bump);
  end function;
  impure function new_pairs(n : natural) return pair_vec is
    variable r : pair_vec(0 to n - 1);
  begin
    for i in r'range loop
      r(i) := (name => "p" & character'val(48 + i), n => c.bump);
    end loop;
    return r;
  end function;
  impure function ids return natural is
  begin
    return c.count;
  end function;
end package body;

use work.elab_values_pkg.all;
entity elab_values_sub is
  generic (m : matrix; p : pair; ps : pair_vec; w : bit_vector; s : string);
end entity;
architecture a of elab_values_sub is
  signal by_m : bit_vector(1 to m(m'high(1), m'low(2)));
begin
  process
  begin
    report "sub m " & integer'image(m'length(1)) & "x" & integer'image(m'length(2))
      & " total " & integer'image(total(m)) & " by_m " & integer'image(by_m'length);
    report "sub p " & p.name & " " & integer'image(p.n) & " ps " & names(ps)
      & " " & integer'image(ps(ps'right).n) & " w " & to_string(w) & " s " & s;
    wait;
  end process;
end architecture;

use work.elab_values_pkg.all;
entity elab_values is
  generic (n : natural := 3);
end entity;
architecture a of elab_values is
  -- Multi-dimensional aggregates that depend on a generic, and calls taking and returning them.
  constant m1 : matrix(0 to 1, 0 to 2) := ((n, 2, 3), (4, 5, n * 2));
  constant m2 : matrix := transpose(m1);
  constant m3 : matrix := (1 to 2 => (0 to n - 2 => n), 3 => (0 to n - 2 => 1));
  signal by_m2 : bit_vector(1 to m2(2, 1) + total(m3));
  -- Impure initial values in declaration order: a record with an array element and an array
  -- of records between two scalar ones.
  constant first : natural := next_id;
  constant p1 : pair := new_pair("ab");
  constant third : natural := next_id;
  constant ps : pair_vec := new_pairs(2);
  constant last : natural := next_id;
  signal by_p1 : bit_vector(1 to p1.n + ps(1).n);
  -- Arrays of composites and records with array elements.
  constant pv : pair_vec := mk_pairs(n);
  constant pw : pair_vec := (swap(pv(1)), (pv(2).name, n), (name => "zz", n => pv(n).n));
  constant ws : word_vec := ("0001", "0110", (3 => '1', others => '0'));
  constant wx : bit_vector := xor_all(ws) xor "0101";
  signal by_pv : bit_vector(1 to pv'length + pw(1).name'length + names(pw)'length);
  -- Shift operators.
  constant b8 : bit_vector(7 downto 0) := "1001" & "0110";
  constant sl : bit_vector := b8 sll n;
  constant sr : bit_vector := b8 srl n;
  constant la : bit_vector := b8 sla 2;
  constant ra : bit_vector := b8 sra 2;
  constant lr : bit_vector := b8 rol n;
  constant rr : bit_vector := b8 ror n;
  signal by_shift : bit_vector(1 to bit'pos(sl(7)) + 2 * bit'pos(rr(0)) + 4 * bit'pos(la(1)));
  -- TO_STRING of real and physical values.
  constant tr : string := to_string(real(n) / 2.0, 2);
  constant tf : string := to_string(1.25 * real(n), "%.3f");
  constant tp : string := to_string(real(n) * 0.5);
  constant tt : string := to_string(n * 1 ns);
  constant tu : string := to_string(n * 1 us, ns);
  constant td : string := to_string(n * 1 cm);
  signal by_text : bit_vector(1 to tr'length + tt'length + td'length);
begin
  u : entity work.elab_values_sub
    generic map (
      m  => transpose(m1),
      p  => swap(p1),
      ps => mk_pairs(n - 1) & pv(1),
      w  => wx ror 1,
      s  => tt & "/" & tr);

  process
    variable r : real := real(n) * 0.5;
  begin
    report "m2 " & integer'image(m2(2, 1)) & " m3 " & integer'image(total(m3))
      & " by_m2 " & integer'image(by_m2'length);
    assert m2 = transpose(m1) severity failure;
    assert m3'length(1) = 3 and m3'length(2) = n - 1 and m3(3, n - 2) = 1 severity failure;
    report "ids first " & integer'image(first) & " p1 " & p1.name & " " & integer'image(p1.n)
      & " third " & integer'image(third) & " ps " & names(ps) & " "
      & integer'image(ps(0).n) & " " & integer'image(ps(1).n)
      & " last " & integer'image(last) & " count " & integer'image(ids)
      & " by_p1 " & integer'image(by_p1'length);
    report "pv " & names(pv) & " pw " & names(pw) & " " & integer'image(pw(0).n)
      & " " & integer'image(pw(1).n) & " " & integer'image(pw(2).n)
      & " wx " & to_string(wx) & " by_pv " & integer'image(by_pv'length);
    assert pv = mk_pairs(n) severity failure;
    assert ws(1) = "0110" severity failure;
    report "shifts " & to_string(sl) & " " & to_string(sr) & " " & to_string(la) & " "
      & to_string(ra) & " " & to_string(lr) & " " & to_string(rr)
      & " by_shift " & integer'image(by_shift'length);
    assert sl = (b8 sll n) and sr = (b8 srl n) and lr = (b8 rol n) and rr = (b8 ror n)
      severity failure;
    report "text " & tr & " " & tf & " " & tt & " " & tu & " " & td
      & " by_text " & integer'image(by_text'length);
    assert tp = to_string(r) severity failure;
    wait;
  end process;
end architecture;
