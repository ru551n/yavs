-- difftest: top=attr_length_2019 std=2019
-- LCS-2016-018: 'length of a discrete type or subtype is its number of values (0 for a null
-- range), also for a subtype whose bounds are known only at elaboration.
package pack018 is
  type thing_t is (one, two, red, blue);
  type thingmap_t is array (thing_t range <>) of integer;
  signal thingmap_num : thingmap_t(one to two);
  signal thingmap_colors : thingmap_t(blue downto red);
  constant size : natural := thing_t'length;
  signal rev_copy : thingmap_t(thingmap_colors'reverse_range);
end package;

use work.pack018.all;
entity attr_length_2019 is
  generic (n : integer := 4);
end entity;
architecture t of attr_length_2019 is
  type color is (red, green, blue);
  subtype c2 is color range green to blue;
  subtype i5 is integer range 3 to 7;
  subtype d5 is integer range 7 downto 3;
  subtype null_i is integer range 5 to 1;
  subtype s is integer range 1 to n;
  subtype d is integer range n downto 1;
  type arr is array (0 to color'length - 1) of integer;
begin
  process
    variable v : integer range 0 to n * 2;
  begin
    report "col " & integer'image(color'length) & integer'image(c2'length) & integer'image(arr'length);
    report "int " & integer'image(i5'length) & integer'image(d5'length) & integer'image(null_i'length);
    report "base " & integer'image(bit'length) & integer'image(boolean'length) & integer'image(character'length);
    report "pack " & integer'image(size) & integer'image(thingmap_num'length) & " " & thing_t'image(rev_copy'left);
    report "dyn " & integer'image(s'length) & integer'image(d'length) & integer'image(v'subtype'length);
    assert thing_t'length = thing_t'pos(thing_t'right) + 1 severity failure;
    wait;
  end process;
end architecture;
