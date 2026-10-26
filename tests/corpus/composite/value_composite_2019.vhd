-- difftest: top=value_composite_2019 std=2019
-- T'VALUE of a composite parses the same literal forms as T'IMAGE writes (LRM 16.2).
library ieee;
use ieee.std_logic_1164.all;

entity value_composite_2019 is end entity;
architecture a of value_composite_2019 is
  type pair_t is record x, y : integer; end record;
  type abc is ('a', 'b', 'c');
  type abc_vec is array (natural range <>) of abc;
  subtype abc3 is abc_vec(3 downto 1);
  subtype int3 is integer_vector(1 to 3);
  type record_with_array is record a : int3; b : boolean; end record;
  type matrix is array (natural range <>, natural range <>) of integer;
  subtype mat22 is matrix(1 to 2, 3 to 4);
  type rec_t is record x : integer; b : boolean; end record;
  type rec_vec is array (natural range <>) of rec_t;
  subtype rec2 is rec_vec(1 to 2);
  subtype small_int is integer range -2 to 2;
  type small_vec is array (natural range <>) of small_int;
  subtype small3 is small_vec(1 to 3);
begin
  process
    variable p : pair_t;
    variable s : abc_vec(1 to 3);
    variable s3 : abc3;
    variable iv : int3;
    variable rv : record_with_array;
    variable m : mat22;
    variable r : rec2;
    variable sm : small3;
    variable bv : bit_vector(3 downto 0);
    variable sl : std_logic_vector(1 to 4);
  begin
    -- These two are the issue's reproducer and also run during elaboration.
    assert pair_t'value("(1,2)") = (1,2) report "record" severity failure;
    assert abc_vec'value("abc") = "abc" report "open array" severity failure;
    assert abc_vec'value("abc")'left = 0 and abc_vec'value("abc")'right = 2
      report "open bounds" severity failure;

    p := pair_t'value("(x => 4, y => -2)");
    assert p = (4,-2) report "named record" severity failure;
    s := abc_vec'value("abc");
    assert s = "abc" report "array" severity failure;
    s3 := abc3'value("(3 => 'c', others => 'a')");
    assert s3 = ('c','a','a') and s3'left = 3 and s3'right = 1
      report "constrained named aggregate" severity failure;
    iv := int3'value("(1, -2, 3)");
    assert iv = (1,-2,3) report "integer array" severity failure;
    rv := record_with_array'value("(a => (4,5,6), b => true)");
    assert rv.a = (4,5,6) and rv.b report "record with array" severity failure;
    m := mat22'value("((1,2),(3,4))");
    assert m = ((1,2),(3,4)) report "matrix" severity failure;
    r := rec2'value("((x => 1, b => true),(2,false))");
    assert r(1) = (1,true) and r(2) = (2,false) report "array of records" severity failure;
    sm := small3'value("(-2,0,2)");
    assert sm = (-2,0,2) report "narrow integer elements" severity failure;
    bv := bit_vector'value("1010");
    assert bv = "1010" report "bit planes" severity failure;
    sl := std_logic_vector'value("10XZ");
    assert sl = "10XZ" report "logic planes" severity failure;
    report "ok";
    wait;
  end process;
end architecture;
