-- rsim-test: std=2008
-- expect: E1002 4:19 unit `foo` not found in library `ieee`
-- LRM 3.3
architecture a of ieee.foo is
begin
end architecture;
