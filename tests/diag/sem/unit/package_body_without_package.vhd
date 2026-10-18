-- yavs-test: std=2008
-- expect: E1004 4:14 package `nopkg` not found; a package body needs a primary unit
-- LRM 4.8
package body nopkg is
end package body;
