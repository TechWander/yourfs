/*
   Copyright (C) by Jun Han <hanjun@nao.cas.cn> 2023
*/

#define _FILE_OFFSET_BITS 64

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <getopt.h>
#include <poll.h>
#include <pthread.h>
#include <unistd.h>
#include <sys/types.h>
#include <stdbool.h>
#include "../config.h"
#include "../include/yfs_scan.h"

#if defined(SQLITE3)
#include "../include/yfs_sqlite3.h"
#endif

#if defined(POSTGRES)
#include "../include/yfs_postgres.h"
#endif


#if defined(PASSWD)
#include "../include/yfs_passwd.h"
#endif

void print_usage(char *name)
{
	printf("Yourfs_tool Usage(v1.0) : %s \n",name);

	printf( "\t [-?|--help] \n"
			"\nyourfs_tool options : \n"
			"\t [-s scan_path] Setup path to scan. if not set, this routine used to write database. \n"
			"\t [-o txt name] The output txt-name when scan dir path. \n"
		        "\n"
			"\t [-F database_type ] It is used to filter database type to write.(you should compile sqlite3,postgres module to support.)\n"
			"\t [-i txt name] The input txt-name when writing database. \n"
			"\t [-X database ]\n"
                        #if defined(SQLITE3)
			"\t\tsqlite3:  /path/data.db\n"
                        #endif
                        #if defined(POSTGRES)
			"\t\tpostgres: ip,port,user,passwd,dbname\n"
			#endif
                        "\t [-Z tablename ] \n"
			#if defined(SQLITE3)
			"\t [-M MB ] It is used to setup sqlite3 cache.\n"
			#endif
		
			
                        #if defined(PASSWD)
                        "\t [-P passwd ] \n"
                        #endif
                        "\n\t (If you have any question or bug report, please connect hanjun@nao.cas.cn) \n"
			);
	exit(0);
}

int main(int argc, char *argv[])
{
	static struct option long_opts[] = {
		{ "help", no_argument, 0, '?' },
		{"scan",no_argument,0,'s'},
                {"input",required_argument,0,'i'},
		{"output",required_argument,0,'o'},
		{"database",required_argument,0,'X'},
		{"table",required_argument,0,'Z'},
		{"cache_memory",required_argument,0,'M'},
		{"filter_database",required_argument,0,'F'},
		{"passwd",required_argument,0,'P'},
		{ NULL, 0, 0, 0 }
	};

	int    cache_memory = 20000,opt_idx = 0,c;
	bool   create_database = false;
	char   database_type[32]="",database[1024]="",output_filename[256]="output.txt",scan_path[256]="",table[256]="data",input_filename[256]="input.txt";
	char   ip[32],port[10],username[32],passwd[32],dbname[32],fs_key[128]="hanjun@nao.cas.cn";

	while ((c = getopt_long(argc, argv, "?s:Do:X:Z:M:F:i:P:", long_opts, &opt_idx)) > 0) {
		switch (c) {
		
		case '?':
			print_usage(argv[0]);
			return 0;
		case 's':
			strcpy(scan_path,optarg);
			break;
	        case 'o':
			strcpy(output_filename,optarg);
			break;
		case 'i':
                        strcpy(input_filename,optarg);
                        break;
		case 'X':
		        strcpy(database,optarg);
			break;
		case 'Z':
			strcpy(table,optarg);
			break;
                case 'M':
                        cache_memory = (int)(atof(optarg)*1000);
                        break;
		case 'F':
                        strcpy(database_type,optarg);
                        break;
		case 'P':
			strcpy(fs_key,optarg);
			break;
		}
	}

       // 
       if(argc == 1) {
            print_usage(argv[0]);
            goto finished;
        }

       #if defined(PASSWD)
       yfs_check_pass(fs_key); 
       #endif

       // file-output
       if(strcmp(scan_path,"")!=0) {
	  int replace_num = strlen(scan_path);     
	  char pre[512];
	  FILE *fp = fopen(output_filename,"w");
	  memcpy(pre,scan_path,strlen(scan_path)+1);
	  TreeNode *n = createTree(scan_path,replace_num,pre,fp);
	  fclose(fp);
          return 0;
        }
       
       // write sqlite3
       if(strcmp(database_type,"sqlite3")==0) {
	    #if defined(SQLITE3)
            yfs_sqlite3_create_table(database,table);
	    yfs_sqlite3_write_table(database,table,input_filename,cache_memory);
            #endif
          }	      


       // write postgres
       if(strcmp(database_type,"postgres")==0) {
            #if defined(POSTGRES)
            sscanf(database, "%[^,],%[^,],%[^,],%[^,],%s",ip, port, username,passwd,dbname);
            yfs_postgres_create_table(ip, port, username, passwd, dbname, table);
	    yfs_postgres_write_table(ip, port, username, passwd, dbname, table,input_filename);
            #endif
          }



finished:
	return 0;

}


