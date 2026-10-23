use std.textio.all;
entity top is end;
architecture a of top is
begin
  process
    variable l : line;
    variable y : bit_vector(7 downto 0) := (others => '0');
    variable z : bit_vector(3 downto 0) := (others => '0');
    variable good : boolean;
  begin
    l := new string'("1 1 0 0 0 0 1 1");
    read(l, y, good);
    report "RES spaces " & boolean'image(good) & " " & to_string(y);
    l := new string'("  1100");
    read(l, z, good);
    report "RES leading " & boolean'image(good) & " " & to_string(z);
    l := new string'("1" & HT & "10 1");
    z := "0000";
    read(l, z, good);
    report "RES tab " & boolean'image(good) & " " & to_string(z);
    l := new string'("1 1x0");
    z := "0000";
    read(l, z, good);
    report "RES bad " & boolean'image(good);
    wait;
  end process;
end;
