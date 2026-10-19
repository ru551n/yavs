-- yavs-test: std=2019
-- expect: E1621 13:14 attribute `'image` does not apply to a type mark of type `r`
-- expect: E1621 14:14 attribute `'image` does not apply to an object of type `r`
-- LRM 2019 16.2.2: 'IMAGE needs a type whose elements all have an image (no access)
entity e is end entity;
architecture a of e is
  type ip is access integer;
  type r is record p : ip; end record;
begin
  process
    variable v : r;
  begin
    report r'image(v);
    report v'image;
    wait;
  end process;
end architecture;
