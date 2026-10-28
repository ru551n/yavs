-- yavs-test: std=2008
-- expect: E1238 8:12 a variable declared here must be a shared variable
-- expect: E1238 11:14 a variable declared here must be a shared variable
-- LRM 6.4.2.4: outside a process, subprogram or protected type a variable is shared (#811)
entity e is
end entity;
architecture a of e is
  variable v : integer;
begin
  b : block
    variable w : integer;
  begin
  end block;
  process
    procedure p is
      variable ok2 : integer;
    begin end;
  begin
    wait;
  end process;
end architecture;
