-- yavs-test: std=2008
-- expect: none
-- LRM 7.2: a label is decorated in the declarative part of the construct whose statement part holds it;
-- a port of the entity in its architecture (12.1: one declarative region), as sim1, sim2 and sim3 accept
entity e is
  port (port_data : in bit);
end entity;
architecture a of e is
  attribute keep : boolean;
  attribute keep of port_data : signal is true;
  attribute kh : string;
  attribute kh of u1, blk : label is "TRUE";
  signal s : bit;
begin
  u1 : s <= port_data;
  blk : block is
    attribute kh of p : label is "FALSE";
  begin
    p : process is
    begin
      wait;
    end process;
  end block;
end architecture;
