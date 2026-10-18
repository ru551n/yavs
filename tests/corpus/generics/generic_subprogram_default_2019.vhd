-- difftest: top=generic_subprogram_default_2019 std=2019
-- Defaults of an entity's generic subprograms: `is <>` is looked up at the instance.
entity show is
  generic (type t; v : t; function img(x : t) return string is <>);
end;
architecture a of show is
begin
  process begin report img(v); wait; end process;
end;
entity generic_subprogram_default_2019 is end;
architecture t of generic_subprogram_default_2019 is
  function img(x : integer) return string is begin return "int " & integer'image(x); end;
  function img(x : boolean) return string is begin return "bool " & boolean'image(x); end;
begin
  u1 : entity work.show generic map (t => integer, v => 4);
  u2 : entity work.show generic map (t => boolean, v => true);
end;
