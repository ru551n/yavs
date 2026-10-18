-- difftest: top=composite_eval std=2008
-- Composite values evaluated during elaboration (#77): aggregates, records, indexing,
-- slices, concatenation, composite equality, and function calls with array and record
-- arguments and results, used as generics, signal bounds, generate ranges and conditions.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package composite_eval_pkg is
  type int_vec is array (natural range <>) of integer;
  type rec is record a : integer; b : character; end record;
  function ones(v : bit_vector) return natural;
  function mk(n : natural) return bit_vector;
  function swap(r : rec) return rec;
  function rev(v : int_vec) return int_vec;
  constant tab : int_vec := (3, 5, 7);
  constant named : int_vec(1 to 4) := (2 => 9, 1 | 3 => 4, others => 0);
  constant r0 : rec := (a => 4, b => 'x');
end;
package body composite_eval_pkg is
  function ones(v : bit_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop if v(i) = '1' then n := n + 1; end if; end loop;
    return n;
  end;
  function mk(n : natural) return bit_vector is
    variable r : bit_vector(n - 1 downto 0) := (others => '1');
  begin return r; end;
  function swap(r : rec) return rec is
  begin return (a => -r.a, b => 'y'); end;
  function rev(v : int_vec) return int_vec is
    variable r : int_vec(v'range);
  begin
    for i in v'range loop r(i) := v(v'high - i + v'low); end loop;
    return r;
  end;
end;
use work.composite_eval_pkg.all;
entity composite_eval_sub is generic (v : bit_vector; w : natural; s : string); port (o : out natural); end;
architecture a of composite_eval_sub is begin
  o <= ones(v) + w + s'length;
end;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
use work.composite_eval_pkg.all;
entity composite_eval is end;
architecture t of composite_eval is
  constant k : natural := ones("10110");
  constant m : bit_vector := mk(k);
  constant u : std_logic_vector(7 downto 0) := std_logic_vector(to_unsigned(200, 8));
  constant n : natural := to_integer(unsigned(u(7 downto 4)));
  constant r1 : rec := swap(r0);
  constant t2 : int_vec := rev(tab & 11);
  constant same : boolean := m = "111" and tab(0 to 1) = (3, 5) and (m xor "101") = "010";
  signal s : bit_vector(tab(1) downto 0);
  signal o1, o2 : natural;
begin
  i1 : entity work.composite_eval_sub generic map (v => mk(3), w => r0.a, s => "ab" & r1.b) port map (o1);
  i2 : entity work.composite_eval_sub generic map (v => m & "0", w => named(2), s => (1 to 3 => 'z')) port map (o2);
  g : for j in tab'range generate
    process begin report "g " & integer'image(tab(j)) & " " & integer'image(t2(j)); wait; end process;
  end generate;
  c : if same and r1.a = -4 generate
    process begin report "same"; wait; end process;
  end generate;
  process begin
    wait for 1 ns;
    report "k=" & integer'image(k) & " len=" & integer'image(m'length) & " n=" & integer'image(n)
      & " s=" & integer'image(s'length) & " o1=" & integer'image(o1) & " o2=" & integer'image(o2)
      & " t2=" & integer'image(t2'length) & " r1=" & integer'image(r1.a) & r1.b;
    wait;
  end process;
end;
