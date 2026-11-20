-- yavs-test: std=2019
-- expect: E1300 12:29 a record with left and right fields is no range record
-- LRM 5.2.1 (2019): only a value of a range record type ('RECORD) is a range
entity e is end entity;
architecture a of e is
  subtype ir is integer'range'record;
  type plain is record left, right : integer; end record;
  function h(k : integer) return plain is begin return (k, k); end function;
begin
  process
    variable v : bit_vector(0 to 3);
    variable w : bit_vector(h(1));
  begin
    wait;
  end process;
end architecture;
