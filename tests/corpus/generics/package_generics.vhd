-- difftest: top=package_generics std=2008
package pk is
  generic (n : integer := 3);
  function twice return integer;
end package;
package body pk is
  function twice return integer is begin return 2 * n; end;
end package body;
package pk5 is new work.pk generic map (n => 5);
package pk_default is new work.pk;
entity package_generics is end;
architecture t of package_generics is
begin
  process begin
    report integer'image(work.pk5.twice) & " " & integer'image(work.pk_default.twice);
    wait;
  end process;
end;
