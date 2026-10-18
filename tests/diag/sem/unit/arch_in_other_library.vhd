-- yavs-test: std=2008
-- expect: E1002 5:19 design unit `foo` not found in library `ieee`
-- LRM 3.3.1
library ieee;
architecture a of ieee.foo is
begin
end architecture;
