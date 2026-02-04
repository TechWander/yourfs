#ifndef YOURFS_H
#define YOURFS_H  

#include <stdbool.h>
#include <fuse.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "./yfs_define.h"
#include "../config.h"

#if defined(SQLITE3)
#include <sqlite3.h>
#endif

#if defined(POSTGRES)

#if defined(UBUNTU)
#include <postgresql/libpq-fe.h>
#endif

#if defined(ROCKY)
#include <libpq-fe.h>
#endif

#endif


struct yourfs_data
{
  char database[1024];
  char table[128];

  #if defined(SQLITE3)
  sqlite3 *db;
  #endif

  #if defined(POSTGRES)
  PGconn *conn;
  PGresult *res;
  #endif

  void (*readdir)();
  bool (*exists)();
  

};




#endif



