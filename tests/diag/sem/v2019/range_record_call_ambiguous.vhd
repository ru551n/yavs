-- yavs-test: std=2019
-- expect: E1302 13:14 two overloads return range records of different types
-- LRM 12.5: a range in a loop has exactly one interpretation
entity e is end entity;
architecture a of e is
  subtype ir is integer'range'record;
  subtype cr is character'range'record;
  function amb(k : integer) return ir is begin return (left => k, right => k, direction => ascending); end function;
  function amb(k : integer) return cr is begin return (left => 'a', right => 'b', direction => ascending); end function;
begin
  process
  begin
    for i in amb(1) loop
    end loop;
    wait;
  end process;
end architecture;
