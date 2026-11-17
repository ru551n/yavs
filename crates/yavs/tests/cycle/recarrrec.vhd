-- Records holding arrays of records, as register-file converters make them (`W` replaced by a
-- width): a function fills a local of such a record from words, field by field at static and
-- loop indices and one element at a run-time index, with an index from a field of one of its
-- elements, and a local without an initial value starts at the `'left` of its fields; against the
-- normal mode.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package cycle_recarrrec_pkg is
  type flags_t is record
    valid : std_ulogic;
    tag : unsigned(W - 1 downto 0);
  end record;
  type mode_t is (idle, run, halt);
  type buf_t is record
    flags : flags_t;
    base : std_ulogic_vector(3 downto 0);
    cnt : integer range -5 to 9;
    down : integer range 9 downto 2;
    mode : mode_t;
  end record;
  type buf_vec_t is array (0 to 2) of buf_t;
  type down_t is record
    hi : std_ulogic_vector(W - 1 downto 0);
    buf : buf_vec_t;
    ctrl : std_ulogic;
  end record;
  constant down_init : down_t := (
    hi => (others => '0'),
    buf => (0 => (flags => ('0', (others => '1')), base => "0101", cnt => 3, down => 4, mode => run),
            1 => (flags => ('1', (others => '0')), base => "0011", cnt => -2, down => 5, mode => idle),
            2 => (flags => ('0', (others => '1')), base => "1001", cnt => 7, down => 6, mode => run)),
    ctrl => '1');
  subtype word_t is std_ulogic_vector(W + 7 downto 0);
  type words_t is array (0 to 3) of word_t;
  function to_down(data : words_t) return down_t;
  function fresh(k : natural) return down_t;
  function show(d : down_t) return string;
  function ints(d : down_t) return string;
end package;

package body cycle_recarrrec_pkg is
  function to_down(data : words_t) return down_t is
    variable result : down_t := down_init;
  begin
    result.hi := data(0)(W - 1 downto 0);
    -- An index from a field of an element of the local (6, from its initial value).
    result.ctrl := data(3)(W + result.buf(2).down - 6);
    for i in 0 to 2 loop
      result.buf(i).flags.valid := data(i + 1)(0);
      result.buf(i).flags.tag := unsigned(data(i + 1)(W + 7 downto 8));
      result.buf(i).base := data(i + 1)(4 downto 1);
      result.buf(i).cnt := to_integer(unsigned(data(i + 1)(7 downto 5))) - 5;
    end loop;
    if data(0)(0) = '1' then
      result.buf(2).mode := halt;
      result.buf(0).down := 9;
    end if;
    result.buf(1) := result.buf(to_integer(unsigned(data(3)(1 downto 0))) mod 3);
    return result;
  end function;

  function fresh(k : natural) return down_t is
    variable r : down_t;
  begin
    r.buf(k mod 3).base := "1111";
    r.buf(2 - k mod 3).cnt := k mod 7;
    return r;
  end function;

  function show(d : down_t) return string is
  begin
    return to_hstring(d.hi) & " " & std_ulogic'image(d.ctrl)
      & " " & std_ulogic'image(d.buf(0).flags.valid) & to_hstring(d.buf(0).flags.tag)
      & " " & to_hstring(d.buf(1).base) & integer'image(d.buf(1).cnt) & integer'image(d.buf(1).down)
      & " " & mode_t'image(d.buf(2).mode) & to_hstring(d.buf(2).flags.tag) & integer'image(d.buf(0).down);
  end function;

  -- Its logic fields start at 'U', which cycle mode reads as 0: only the integers and enumerations.
  function ints(d : down_t) return string is
  begin
    return integer'image(d.buf(0).cnt) & integer'image(d.buf(1).cnt) & integer'image(d.buf(2).cnt)
      & integer'image(d.buf(0).down) & integer'image(d.buf(2).down) & " " & mode_t'image(d.buf(1).mode);
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_recarrrec_pkg.all;
entity cycle_recarrrec is
end entity;
architecture a of cycle_recarrrec is
  signal words : words_t := (others => (others => '0'));
  signal down, other : down_t;
  signal k : natural := 0;
begin
  conv : process (words)
  begin
    down <= to_down(words);
  end process;

  other <= fresh(k);

  main : process
    variable v : word_t;
  begin
    for n in 1 to 12 loop
      for j in 0 to 3 loop
        for b in v'range loop
          if (n * 31 + j * 7 + b * 13) mod 5 < 2 then
            v(b) := '1';
          else
            v(b) := '0';
          end if;
        end loop;
        words(j) <= v;
      end loop;
      k <= n;
      wait for 5 ns;
      report "k=" & integer'image(n) & " d=" & show(down) & " o=" & ints(other);
    end loop;
    wait;
  end process;
end architecture;
