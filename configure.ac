AC_PREREQ(2.50)
AC_INIT([yourfs], [1.0.0], [hanjun@nao.cas.cn])
AM_INIT_AUTOMAKE([-Wall foreign])
AC_CANONICAL_HOST
AC_CONFIG_MACRO_DIR([m4])
m4_pattern_allow([AM_PROG_AR])
AM_PROG_AR

# Work around stupid autoconf default cflags. pt 1
SAVE_CFLAGS="x${CFLAGS}"

AC_PROG_CC
AC_PROG_LIBTOOL

AM_PROG_CC_C_O
PKG_PROG_PKG_CONFIG

# Work around stupid autoconf default cflags. pt 2
if test "$SAVE_CFLAGS" = "x"; then
  CFLAGS=""
fi

# We always want 64 bit file offsets
CFLAGS="${CFLAGS} -D_FILE_OFFSET_BITS=64 -std=gnu99"

if test "$ac_cv_prog_gcc" = yes; then
   WARN_CFLAGS="-Wall -Werror -Wshadow -Wno-write-strings -Wstrict-prototypes -Wpointer-arith -Wcast-align -Wno-strict-aliasing"
fi

AC_SUBST(WARN_CFLAGS)

AC_CONFIG_HEADER(config.h)

AC_HEADER_ASSERT
AC_CHECK_HEADER([fuse.h], [], [AC_MSG_ERROR([fuse.h is missing. You need to install libfuse-dev])], [])
AC_CHECK_HEADER([nfsc/libnfs.h], [], [AC_MSG_ERROR([libnfs.h is missing. You need to install libnfs-dev])], [])

AC_CACHE_CHECK([for st_atim support],libiscsi_cv_HAVE_ST_ATIM,[
AC_TRY_COMPILE([
#include <sys/types.h>
#include <sys/stat.h>
#include <unistd.h>],
[struct stat st; st.st_atim.tv_sec = 0;],
libiscsi_cv_HAVE_ST_ATIM=yes,libiscsi_cv_HAVE_ST_ATIM=no)])
if test x"$libiscsi_cv_HAVE_ST_ATIM" = x"yes"; then
    AC_DEFINE(HAVE_ST_ATIM,1,[Whether we have st_atim support])
fi

AC_SEARCH_LIBS([fuse_get_context], [fuse], [], [
  AC_MSG_ERROR([fuse library unavailable])
])


# compile postgres module
AC_ARG_WITH([postgres],
  [AS_HELP_STRING([--with-postgres], [set up to compile and support postgres.])],
  [], [with_postgres=no])
AC_DEFINE_UNQUOTED([WITH_POSTGRES], ["$with_postgres"], [Compile and support postgres])



# compile platform support
AC_ARG_WITH([plat],
  [AS_HELP_STRING([--with-plat], [set up  platform, ubuntu,rocky])],
  [], [with_ubuntu=no])
AC_DEFINE_UNQUOTED([WITH_PLAT], ["$with_plat"], [Support platform ])


if test "x$with_plat" == "xubuntu"; then
        AC_DEFINE(UBUNTU, 1, Support ubuntu)
        # the path in ubuntu
        if test "x$with_postgres" == "xyes"; then
            AC_CHECK_HEADER([postgresql/libpq-fe.h], [], [AC_MSG_ERROR([libpq-fe.h not found.])], [])
            AC_SUBST([POSTGRES], ["-lpq"])
            AC_DEFINE(POSTGRES, 1, Compile and support postgres)
        fi
        # the path in rocky linux   
fi

if test "x$with_plat" == "xrocky"; then
        AC_DEFINE(ROCKY, 1, Support rocky)
        # the path in rocky linux          
        if test "x$with_postgres" == "xyes"; then
            AC_CHECK_HEADER([libpq-fe.h], [], [AC_MSG_ERROR([libpq-fe.h not found.])], [])
            AC_SUBST([POSTGRES], ["-lpq"])
            AC_DEFINE(POSTGRES, 1, Compile and support postgres)
        fi 
fi




# compile passwd support module
AC_ARG_WITH([passwd],
  [AS_HELP_STRING([--with-passwd], [set up to support password])],
  [], [with_passwd=no])
AC_DEFINE_UNQUOTED([WITH_PASSWD], ["$with_passwd"], [Support passwd])


if test "x$with_passwd" == "xyes"; then
        AC_DEFINE(PASSWD, 1, Support passwd)
fi





# compile sqlite3 module
AC_ARG_WITH([sqlite3],
  [AS_HELP_STRING([--with-sqlite3], [set up to compile and support sqlite3.])],
  [], [with_sqlite3=no])
AC_DEFINE_UNQUOTED([WITH_SQLITE3], ["$with_sqlite3"], [Compile and support sqlite3])

if test "x$with_sqlite3" == "xyes"; then
	AC_CHECK_HEADER([sqlite3.h], [], [AC_MSG_ERROR([sqlite3.h not found.])], [])
	AC_SUBST([SQLITE3], ["-lsqlite3"])
	AC_DEFINE(SQLITE3, 1, Compile and support sqlite3)	
fi











#output
AC_CONFIG_FILES([Makefile]
                [yourfs/Makefile]
               )

AC_OUTPUT([yourfs.pc])
