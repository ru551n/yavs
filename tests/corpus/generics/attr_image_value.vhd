-- difftest: top=attr_image_value std=2008
-- 'image and 'value of constants and literals in declarations and generic maps are evaluated
-- while the design is elaborated (LRM 16.2.1, 14.4).
entity leaf is
  generic (name : string; t : time);
end entity;
architecture a of leaf is
begin
  process begin
    report name & " " & time'image(t);
    wait;
  end process;
end architecture;

entity attr_image_value is end entity;
architecture a of attr_image_value is
  constant n : integer := 42;
  signal s : string(1 to integer'image(n)'length) := integer'image(n);
  signal t : time := time'value("123 ns");
  signal i : integer := integer'value(" -7");
  signal b : boolean := boolean'value("TRUE");
begin
  u : entity work.leaf generic map (name => "leaf" & integer'image(n), t => time'value("5 us"));
  process begin
    report s & " " & time'image(t) & " " & integer'image(i) & " " & boolean'image(b);
    wait;
  end process;
end architecture;
