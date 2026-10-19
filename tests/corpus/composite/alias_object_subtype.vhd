-- difftest: top=alias_object_subtype std=2008
-- 'subtype of an object whose bounds come from a port (#315): a variable declared as
-- port'subtype, an alias of one of its record elements declared with that element's
-- 'subtype (and without a subtype), a variable constrained by the alias's element range,
-- an alias of an array of records, and in the architecture a signal, a signal alias and a
-- constant over the port element's 'subtype.
package alias_object_subtype_pkg is
  type rd_t is record
    data : bit_vector;
    resp : bit_vector(1 downto 0);
  end record;
  type rd_arr_t is array (natural range <>) of rd_t;
  type bus_t is record
    rd   : rd_t;
    many : rd_arr_t;
  end record;
end package;

use work.alias_object_subtype_pkg.all;
entity alias_object_subtype_m is
  port (b : inout bus_t);
end entity;

architecture a of alias_object_subtype_m is
  signal sx : b.rd'subtype;
  alias ar : b.rd'subtype is b.rd;
  constant w : natural := ar.data'length;
  signal sy : bit_vector(sx.data'range);
begin
  process
    variable defs : b'subtype;
    alias lrd : defs.rd'subtype is defs.rd;
    alias plain : rd_t is defs.rd;
    alias lm : defs.many'subtype is defs.many;
    variable exp : bit_vector(lrd.data'range);
  begin
    lrd.data := x"a5";
    lm(1).data := x"3c";
    report "v " & integer'image(exp'left) & " " & integer'image(exp'right) & " "
      & integer'image(plain.data'length) & " " & to_string(defs.rd.data) & " "
      & integer'image(lm'left) & " " & integer'image(lm(1).data'left) & " "
      & to_string(defs.many(1).data);
    assert exp'length = 8 severity failure;
    report "s " & integer'image(sx.data'left) & " " & integer'image(ar.data'left) & " "
      & integer'image(w) & " " & integer'image(sy'left);
    wait;
  end process;
end architecture;

use work.alias_object_subtype_pkg.all;
entity alias_object_subtype is
end entity;

architecture t of alias_object_subtype is
  signal s : bus_t(rd(data(7 downto 0)), many(0 to 1)(data(15 downto 8)));
begin
  m : entity work.alias_object_subtype_m port map (b => s);
end architecture;
