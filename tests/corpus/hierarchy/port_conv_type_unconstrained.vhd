-- difftest: top=port_conv_type_unconstrained std=2008 stop=20ns ref=hand
-- Conversions on unconstrained ports (LRM 5.3.2.2, #262): the formal takes its index ranges
-- from the constrained type mark of a type conversion in the actual part (`i => w4(u)`), from
-- the type mark or the parameter subtype of a conversion in the formal part (`u4(o) => s`,
-- `f(o) => s`), also for arrays of records and records with unconstrained elements.
-- ref=hand: the reference simulator crashes on the actual-part and composite forms; the
-- expected output is the LRM result (the third reference simulator agrees on the actual part).
package port_conv_type_unconstrained_pkg is
  type rec is record a : bit; n : integer; end record;
  type ra is array (natural range <>) of rec;
  type rb is array (natural range <>) of rec;
  subtype rb3 is rb(1 to 3);
  subtype ra3 is ra(7 downto 5);
  type ur is record v : bit_vector; k : integer; end record;
  subtype ur4 is ur(v(0 to 3));
  subtype b4 is bit_vector(7 downto 4);
  function img(x : rb) return string;
  function f(x : b4) return bit_vector;
end;
package body port_conv_type_unconstrained_pkg is
  function img(x : rb) return string is
    variable s : string(1 to x'length * 3);
  begin
    for i in 0 to x'length - 1 loop
      s(3 * i + 1 to 3 * i + 3) := bit'image(x(x'left + i).a)(2) & integer'image(x(x'left + i).n)(1) & " ";
    end loop;
    return s;
  end;
  function f(x : b4) return bit_vector is begin return x; end;
end;
library ieee; use ieee.numeric_bit.all;
use work.port_conv_type_unconstrained_pkg.all;
entity port_conv_type_unconstrained_in is
  generic (d : time);
  port (i : in bit_vector; p : in rb; u : in ur);
end;
architecture a of port_conv_type_unconstrained_in is
begin
  process begin
    wait for d;
    report "i " & integer'image(i'left) & " to " & integer'image(i'right) & ": " & to_string(i);
    report "p " & integer'image(p'left) & " to " & integer'image(p'right) & ": " & img(p);
    report "u.v " & integer'image(u.v'left) & " to " & integer'image(u.v'right) & ": " & to_string(u.v);
    wait;
  end process;
end;
entity port_conv_type_unconstrained_out is
  generic (d : time);
  port (o : out bit_vector);
end;
architecture a of port_conv_type_unconstrained_out is
begin
  process begin
    wait for d;
    report "o " & integer'image(o'left) & " " & integer'image(o'right);
    o <= (o'range => '1');
    o(o'left) <= '0';
    wait;
  end process;
end;
use work.port_conv_type_unconstrained_pkg.all;
entity port_conv_type_unconstrained_rout is
  generic (d : time);
  port (q : out rb);
end;
architecture a of port_conv_type_unconstrained_rout is
begin
  process begin
    wait for d;
    report "q " & integer'image(q'left) & " " & integer'image(q'right);
    for k in q'range loop q(k) <= ('1', 5); end loop;
    q(q'left) <= ('0', 1);
    wait;
  end process;
end;
library ieee; use ieee.numeric_bit.all;
use work.port_conv_type_unconstrained_pkg.all;
entity port_conv_type_unconstrained is end;
architecture a of port_conv_type_unconstrained is
  subtype w4 is bit_vector(1 to 4);
  subtype u4 is unsigned(7 downto 4);
  signal u : unsigned(3 downto 0) := "1100";
  signal sa : ra(2 downto 0) := (('1', 1), ('0', 2), ('1', 3));
  signal su : ur(v(3 downto 0)) := ("1001", 7);
  signal s1 : unsigned(3 downto 0);
  signal s2 : bit_vector(3 downto 0);
  signal so : ra(2 downto 0);
begin
  i1 : entity work.port_conv_type_unconstrained_in generic map (1 ns)
    port map (i => w4(u), p => rb3(sa), u => ur4(su));
  o1 : entity work.port_conv_type_unconstrained_out generic map (2 ns) port map (u4(o) => s1);
  o2 : entity work.port_conv_type_unconstrained_out generic map (3 ns) port map (f(o) => s2);
  o3 : entity work.port_conv_type_unconstrained_rout generic map (4 ns) port map (ra3(q) => so);
  process begin
    wait for 5 ns;
    report "s1 " & to_string(s1) & " s2 " & to_string(s2);
    report "so " & bit'image(so(2).a) & integer'image(so(2).n) & bit'image(so(0).a) & integer'image(so(0).n);
    wait;
  end process;
  process (s1) begin
    report "s1 changed " & to_string(s1);
  end process;
end;
