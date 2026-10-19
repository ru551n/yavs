-- difftest: top=record_port_bounds std=2008
-- Bounds of record elements of ports during elaboration (#304): a record port whose
-- unconstrained array elements are constrained by the actual. 'length, 'left, 'right,
-- 'low, 'high and 'range of an element, of a nested record's element and of an element
-- of an array of records, used in entity and architecture constants, signal subtypes,
-- generate ranges and a block constant.
package record_port_bounds_pkg is
  type inner_t is record
    user : bit_vector;
    id   : bit_vector;
  end record;
  type bus_t is record
    resp : inner_t;
    data : bit_vector;
    flag : bit;
  end record;
  type bus_arr_t is array (natural range <>) of bus_t;
end package;

use work.record_port_bounds_pkg.all;
entity record_port_bounds_ctrl is
  port (b : inout bus_t; a : in bus_arr_t);
  constant DW : natural := b.data'length;
  constant UW : natural := b.resp.user'length;
end entity;

architecture a of record_port_bounds_ctrl is
  constant IL : integer := b.resp.id'left;
  constant IR : integer := b.resp.id'right;
  constant AW : natural := a(a'left).resp.id'length;
  signal u : bit_vector(b.resp.user'length - 1 downto 0);
  signal d : bit_vector(b.data'range);
  signal v : bit_vector(a(a'right).data'high downto a(a'right).data'low);
begin
  g : for i in b.resp.id'range generate
    process begin
      report "g " & integer'image(i);
      wait;
    end process;
  end generate;
  blk : block
    constant H : integer := b.data'high + b.resp.user'low;
  begin
    process begin
      report "blk " & integer'image(H);
      wait;
    end process;
  end block;
  process begin
    report "dw " & integer'image(DW) & " uw " & integer'image(UW)
      & " id " & integer'image(IL) & " " & integer'image(IR) & " aw " & integer'image(AW);
    report "u " & integer'image(u'length) & " d " & integer'image(d'left)
      & " " & integer'image(d'right) & " v " & integer'image(v'left) & " " & integer'image(v'right);
    report "run " & integer'image(b.data'length) & " " & integer'image(a(1).resp.user'length);
    wait;
  end process;
end architecture;

use work.record_port_bounds_pkg.all;
entity record_port_bounds is end entity;

architecture t of record_port_bounds is
  signal s : bus_t(resp(user(3 downto 0), id(1 to 5)), data(10 downto 3));
  signal arr : bus_arr_t(0 to 1)(resp(user(6 downto 0), id(2 to 3)), data(9 downto 2));
begin
  c : entity work.record_port_bounds_ctrl port map (b => s, a => arr);
end architecture;
