-- difftest: top=image_composite std=2019 ref=hand
-- ref=hand: the references disagree on the text of composites; the expected output follows
-- the majority of the references that run each form (docs/design/lrm-checks.md, the
-- 'IMAGE and TO_STRING of composites entry; discrepancy composite-image-quoting).
-- 2019 T'IMAGE, X'IMAGE and TO_STRING of arrays and records (LRM 2019 16.2.2, 5.7).
library ieee;
use ieee.std_logic_1164.all;

entity image_composite is
end entity;

architecture a of image_composite is
  type rec_t is record
    x : integer;
    y : bit;
    c : character;
    s : string(1 to 2);
    e : boolean;
  end record;
  type nest_t is record
    r : rec_t;
    v : std_logic_vector(3 downto 0);
  end record;
  type rows_t is array (1 to 2) of bit_vector(1 to 2);
  type mat_t is array (1 to 2, 1 to 2) of integer;
  type mixed_t is (red, 'x', green);
  type mixed_vec_t is array (natural range <>) of mixed_t;
  type recs_t is array (0 to 1) of rec_t;

  constant c_rec   : rec_t := (3, '1', 'a', "hi", true);
  constant c_vec   : integer_vector := (1, 2, 3);
  constant c_null  : integer_vector(1 to 0) := (others => 0);
  constant c_rows  : rows_t := ("01", "10");
  constant c_mat   : mat_t := ((1, 2), (3, 4));
  constant c_mixed : mixed_vec_t := (red, 'x');
  constant c_quote : string := "a""b";
  signal s_nest    : nest_t := ((7, '0', 'z', "ok", false), "01XZ");

  procedure check(got, want : string) is
  begin
    assert got = want report "got " & got & ", want " & want severity failure;
    report got;
  end procedure;

  function img(v : integer_vector) return string is
  begin
    return v'image;
  end function;
begin
  process
    variable v_recs : recs_t := (others => c_rec);
  begin
    check(integer_vector'image(c_vec), "(1,2,3)");
    check(c_vec'image, "(1,2,3)");
    check(img((5, 6)), "(5,6)");
    check(c_null'image, "()");
    check(rec_t'image(c_rec), "(3,'1','a',""hi"",true)");
    check(c_rec'image, "(3,'1','a',""hi"",true)");
    check(to_string(c_rec), "(3,1,a,hi,true)");
    check(rows_t'image(c_rows), "(""01"",""10"")");
    check(to_string(c_rows), "(01,10)");
    check(c_mat'image, "((1,2),(3,4))");
    check(c_mixed'image, "(red,x)");
    check(string'image(c_quote), """a""""b""");
    check(v_recs'image, "((3,'1','a',""hi"",true),(3,'1','a',""hi"",true))");
    check(s_nest'image, "((7,'0','z',""ok"",false),""01XZ"")");
    check(to_string(s_nest), "((7,0,z,ok,false),01XZ)");
    report "image_composite ok";
    wait;
  end process;
end architecture;
