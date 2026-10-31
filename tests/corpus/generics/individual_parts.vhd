-- difftest: top=tb_individual_parts std=2008 ref=hand
-- A generic associated in parts (LRM 6.5.7.1): fields, elements, slices, an unconstrained array
-- and a record element bounded by its parts, through an entity and a component.
-- ref=hand: sim1 stops with "not yet supported" on this generic map, the expected text is sim2's.
package pk_gip is
  type rec is record i : integer; r : real; v : bit_vector(1 to 3); end record;
  type stuff is array (1 to 2) of rec;
  type urec is record a : bit; w : bit_vector; end record;
end package;

use work.pk_gip.all;
entity sub_gip is
  generic (gr : rec; gb : stuff; u : bit_vector; ur : urec; m : bit_vector(0 to 3));
end entity;
architecture a of sub_gip is
  signal s : bit_vector(u'range) := u;
begin
  process begin
    report integer'image(gb(1).i) & " " & integer'image(gb(2).i) & " " & integer'image(gr.i)
      & " " & to_string(gr.v) & " " & to_string(gb(2).v);
    report to_string(u) & " " & integer'image(u'left) & " " & to_string(s) & " " & bit'image(ur.a)
      & " " & to_string(ur.w) & " " & to_string(m);
    wait;
  end process;
end architecture;

use work.pk_gip.all;
entity tb_individual_parts is end entity;
architecture a of tb_individual_parts is
  constant k : natural := 5;
  function f return integer is begin return k + 2; end function;
  component sub_gip is
    generic (gr : rec; gb : stuff; u : bit_vector; ur : urec; m : bit_vector(0 to 3));
  end component;
begin
  u1 : entity work.sub_gip
    generic map (gr.r => 2.5, gr.i => f, gr.v(1) => '1', gr.v(2 to 3) => "01",
                 gb(2) => (890, 135.7, "101"), gb(1).r => 456.7, gb(1).i => k, gb(1).v => "000",
                 u(3) => '1', u(2) => '0', ur.a => '1', ur.w => "10",
                 m(0 to 1) => "01", m(2 to 3) => "10");
  u2 : sub_gip
    generic map (gr => (1, 1.0, "111"), gb(2) => (3, 135.7, "110"), gb(1) => (4, 2.0, "000"),
                 u(0) => '1', u(1) => '0', ur.a => '0', ur.w => "01", m(3) => '1', m(0 to 2) => "101");
end architecture;
