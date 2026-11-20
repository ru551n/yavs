-- yavs-test: std=2019
-- expect: E1300 11:14 a call of a function returning an integer is no range
-- LRM 5.2.1 (2019): a range is a range attribute, two bounds or a value of a range record type
entity e is end entity;
architecture a of e is
  subtype ir is integer'range'record;
  function g(k : integer) return integer is begin return k; end function;
begin
  process
  begin
    for i in g(1) loop
    end loop;
    wait;
  end process;
end architecture;
