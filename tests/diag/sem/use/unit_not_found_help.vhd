-- yavs-test: std=2008
-- expect: E1002 6:10 design unit `numeric_sdt` not found in library `ieee`
-- help: a design unit with a similar name exists: `numeric_std`
-- LRM 12.4
library ieee;
use ieee.numeric_sdt.all;
package q is
end package;
