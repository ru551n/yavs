library ieee; use ieee.std_logic_1164.all;
entity image_text is end;
architecture a of image_text is
  type color is (red, green, blue);
  type cv is array (natural range <>) of color;
  type m2 is array (0 to 1, 0 to 2) of bit;
  type rec is record a : integer; b : bit_vector(3 downto 0); c : string(1 to 3); end record;
  type bvv is array (natural range <>) of bit_vector(1 downto 0);
begin
  process
    variable slv : std_logic_vector(7 downto 0) := "01XZUW-H";
    variable bv  : bit_vector(0 to 10) := "10110011101";
    variable s   : string(1 to 6) := "a""b" & character'val(0) & "cd";
    variable s2  : string(1 to 5) := "hello";
    variable e   : string(1 to 0);
    variable emp : bit_vector(1 to 0);
    variable c   : cv(1 to 3) := (red, blue, green);
    variable mm  : m2 := (('1','0','1'),('0','0','1'));
    variable r   : rec := (5, "1010", "xyz");
    variable bb  : bvv(0 to 1) := ("10", "01");
    variable bo  : boolean_vector(1 to 3) := (true, false, true);
    variable ul  : std_ulogic_vector(2 downto 0) := "1X0";
  begin
    report "slv " & to_string(slv) & " " & slv'image & " " & to_hstring(bv) & " " & to_ostring(bv);
    report "bv " & to_string(bv) & " " & bit_vector'image(bv);
    report "s " & to_string(s2) & " " & string'image(s2);
    report "sq " & string'image(s) & " " & to_string(s);
    report "e [" & to_string(e) & "] " & string'image(e) & " [" & to_hstring(emp) & "] " & to_string(emp);
    report "c " & to_string(c) & " " & cv'image(c);
    report "mm " & to_string(mm) & " " & m2'image(mm);
    report "r " & rec'image(r) & " " & to_string(r);
    report "bb " & bvv'image(bb) & " " & to_string(bb);
    report "bo " & to_string(bo) & " " & boolean_vector'image(bo);
    report "ul " & to_string(ul) & " " & to_hstring(ul) & " " & to_ostring(ul);
    report to_hstring(bit_vector'("1")) & to_hstring(bit_vector'("10101")) & to_ostring(bit_vector'("1")) & to_ostring(bit_vector'("1011")) & to_hstring(bit_vector'("0000"));
    report to_string(character'val(200)) & to_string('a') & boolean'image(true) & color'image(green) & to_string(blue) & integer'image(-5) & real'image(1.5) & time'image(3 ns);
    wait;
  end process;
end;
