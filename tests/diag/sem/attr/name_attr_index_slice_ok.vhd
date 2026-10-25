-- yavs-test: std=2008
-- expect: none
-- LRM 16.2.5, 8.4, 8.5: the name attributes return a string, which can be indexed or sliced
entity e is port (p : out bit); end;
architecture a of e is
begin
  process
    variable c : character;
    variable b : string(1 to 1);
  begin
    c := p'path_name(1);
    c := p'instance_name(1);
    c := e'simple_name(1);
    b := p'simple_name(1 to 1);
    wait;
  end process;
end;
